use std::{
    io,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
};

use anyhow::{Context as _, Result, anyhow, bail};
use tokio::net::TcpListener;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BindMode {
    Exact,
    Walk,
}

pub(crate) fn bind_mode(debug_assertions: bool, port_supplied: bool) -> BindMode {
    if debug_assertions || port_supplied {
        BindMode::Exact
    } else {
        BindMode::Walk
    }
}

pub(crate) fn normalize_host(host: &str) -> Result<&str> {
    let host = host.trim();
    if host.is_empty() {
        bail!("bind host must not be empty");
    }
    if host.starts_with('[') {
        let inner = host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .ok_or_else(|| anyhow!("invalid bind host '{host}'"))?;
        inner
            .parse::<IpAddr>()
            .map_err(|_| anyhow!("invalid bind host '{host}'"))?;
        return Ok(inner);
    }
    if host.parse::<IpAddr>().is_ok() {
        return Ok(host);
    }
    if !host
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
        || host.starts_with('-')
        || host.ends_with('-')
        || host.contains("..")
    {
        bail!("invalid bind host '{host}'");
    }
    Ok(host)
}

pub(crate) fn next_port_on_error(port: u16, error: io::Error, mode: BindMode) -> io::Result<u16> {
    if mode == BindMode::Walk && error.kind() == io::ErrorKind::AddrInUse {
        return port.checked_add(1).ok_or(error);
    }
    Err(error)
}

pub(crate) fn bound_url(addr: SocketAddr) -> String {
    format!("http://{addr}/")
}

pub(crate) async fn bind(host: &str, port: u16, mode: BindMode) -> Result<TcpListener> {
    let host = normalize_host(host)?.to_owned();
    validate_host(&host, port)?;
    let mut port = port;
    loop {
        match TcpListener::bind((host.as_str(), port)).await {
            Ok(listener) => return Ok(listener),
            Err(error) => {
                let kind = error.kind();
                port = next_port_on_error(port, error, mode).with_context(|| {
                    if kind == io::ErrorKind::AddrInUse {
                        format!("address {host}:{port} is already in use")
                    } else {
                        format!("failed to bind {host}:{port}")
                    }
                })?;
            }
        }
    }
}

fn validate_host(host: &str, port: u16) -> Result<()> {
    let mut addrs = (host, port)
        .to_socket_addrs()
        .with_context(|| format!("invalid bind address '{host}:{port}'"))?;
    addrs
        .next()
        .ok_or_else(|| anyhow!("invalid bind address '{host}:{port}'"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn debug_or_explicit_port_is_exact() {
        assert_eq!(bind_mode(true, false), BindMode::Exact);
        assert_eq!(bind_mode(false, true), BindMode::Exact);
        assert_eq!(bind_mode(false, false), BindMode::Walk);
    }

    #[tokio::test]
    async fn walk_skips_an_occupied_port() {
        let held = bind("127.0.0.1", 0, BindMode::Exact).await.unwrap();
        let port = held.local_addr().unwrap().port();
        if port == u16::MAX {
            return;
        }
        let listener = bind("127.0.0.1", port, BindMode::Walk).await.unwrap();
        assert_eq!(listener.local_addr().unwrap().port(), port + 1);
        assert_eq!(
            bound_url(listener.local_addr().unwrap()),
            format!("http://{}:{}/", Ipv4Addr::LOCALHOST, port + 1)
        );
    }
}
