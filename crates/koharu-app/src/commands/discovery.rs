use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr},
    time::Duration,
};

use anyhow::{Context, Result};
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use specta::Type;
use sysinfo::{IpNetwork, Networks};

use super::Error;

static SCAN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const MAX_RESPONSE: usize = 1024 * 1024;

#[derive(Debug, Serialize, Type)]
pub struct LmStudioServer {
    pub base_url: String,
    pub models: Vec<String>,
}

#[derive(Debug, Serialize, Type)]
pub struct LmStudioDiscovery {
    pub subnets: Vec<String>,
    pub servers: Vec<LmStudioServer>,
}

fn lan_subnets(networks: impl IntoIterator<Item = IpNetwork>) -> BTreeSet<Ipv4Addr> {
    networks
        .into_iter()
        .filter_map(|network| {
            let IpAddr::V4(ip) = network.addr else {
                return None;
            };
            if network.prefix != 24 || !ip.is_private() {
                return None;
            }
            let [a, b, c, _] = ip.octets();
            Some(Ipv4Addr::new(a, b, c, 0))
        })
        .collect()
}

fn targets(subnet: Ipv4Addr) -> impl Iterator<Item = Ipv4Addr> {
    let [a, b, c, _] = subnet.octets();
    (1..=254).map(move |host| Ipv4Addr::new(a, b, c, host))
}

#[derive(Deserialize)]
struct ModelList {
    models: Vec<DiscoveredModel>,
}

#[derive(Deserialize)]
struct DiscoveredModel {
    key: String,
    #[serde(rename = "type")]
    kind: ModelKind,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ModelKind {
    Llm,
    Vlm,
    Embedding,
    Decision,
}

async fn probe(client: reqwest::Client, ip: Ipv4Addr, port: u16) -> Option<LmStudioServer> {
    let base_url = format!("http://{ip}:{port}");
    let mut response = client
        .get(format!("{base_url}/api/v1/models"))
        .send()
        .await
        .ok()?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|size| size > MAX_RESPONSE as u64)
    {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    let list: ModelList = serde_json::from_slice(&bytes).ok()?;
    Some(LmStudioServer {
        base_url,
        models: list
            .models
            .into_iter()
            .filter(|model| matches!(model.kind, ModelKind::Llm | ModelKind::Vlm))
            .map(|model| model.key)
            .collect(),
    })
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn discover_lm_studio(port: u16) -> Result<LmStudioDiscovery, Error> {
    if port == 0 {
        return Err(anyhow::anyhow!("LM Studio needs a nonzero port").into());
    }
    let _scan = SCAN
        .try_lock()
        .context("LM Studio discovery is already running")?;
    let subnets = tokio::task::spawn_blocking(|| {
        let networks = Networks::new_with_refreshed_list();
        lan_subnets(
            networks
                .list()
                .values()
                .flat_map(|network| network.ip_networks().iter().copied()),
        )
    })
    .await?;
    // Bound discovery even on machines with many virtual adapters. Never scan
    // public addresses, wider masks, the network address, or broadcast address.
    if subnets.len() > 4 {
        return Err(anyhow::anyhow!(
            "too many private /24 interfaces; enter the server URL manually"
        )
        .into());
    }
    // Discovery never forwards a saved credential to unknown LAN hosts or a proxy.
    let client = reqwest::Client::builder()
        .user_agent(concat!("Koharu/", env!("CARGO_PKG_VERSION")))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_millis(400))
        .timeout(Duration::from_millis(1200))
        .pool_max_idle_per_host(0)
        .build()?;
    let mut servers: Vec<_> = stream::iter(subnets.iter().copied().flat_map(targets))
        .map(|ip| probe(client.clone(), ip, port))
        .buffer_unordered(32)
        .filter_map(|server| async move { server })
        .collect()
        .await;
    servers.sort_by(|left, right| left.base_url.cmp(&right.base_url));
    Ok(LmStudioDiscovery {
        subnets: subnets.into_iter().map(|ip| format!("{ip}/24")).collect(),
        servers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn restricts_discovery_to_private_24_networks_and_host_addresses() {
        let subnets = lan_subnets(
            [
                "192.168.31.23/24",
                "192.168.31.73/24",
                "10.1.2.3/24",
                "192.168.1.5/16",
                "127.0.0.1/24",
                "169.254.1.2/24",
                "8.8.8.8/24",
                "::1/128",
            ]
            .map(|network| network.parse::<IpNetwork>().unwrap()),
        );
        assert_eq!(subnets.len(), 2);
        let hosts: Vec<_> = targets("192.168.31.0".parse().unwrap()).collect();
        assert_eq!(hosts.len(), 254);
        assert_eq!(hosts.first().unwrap().to_string(), "192.168.31.1");
        assert_eq!(hosts.last().unwrap().to_string(), "192.168.31.254");
        assert!(hosts.iter().all(|ip| ip.octets()[..3] == [192, 168, 31]));
    }

    #[tokio::test]
    async fn identifies_native_model_api_and_rejects_unrelated_or_redirected_services() {
        for (status, body, expected) in [
            (
                "200 OK",
                r#"{"models":[{"key":"qwen","type":"llm"},{"key":"embed","type":"embedding"}]}"#,
                true,
            ),
            (
                "200 OK",
                r#"{"data":[{"id":"other-openai-service"}]}"#,
                false,
            ),
            ("302 Found", r#"{"models":[]}"#, false),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let size = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..size]);
                assert!(request.starts_with("GET /api/v1/models "));
                assert!(!request.to_ascii_lowercase().contains("authorization:"));
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            });
            let result = probe(reqwest::Client::new(), Ipv4Addr::LOCALHOST, port).await;
            assert_eq!(result.is_some(), expected);
            if let Some(server) = result {
                assert_eq!(server.models, ["qwen"]);
            }
            server.await.unwrap();
        }
    }
}
