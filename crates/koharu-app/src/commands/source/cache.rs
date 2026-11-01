use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Serialize};
use url::Url;

// Rawkuma rejects Python's default User-Agent with Cloudflare 1010. Its public
// image endpoint accepts Koharu's own identifier; no browser cookies are copied.
const USER_AGENT: &str = concat!("Koharu/", env!("CARGO_PKG_VERSION"));
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct CachedImage {
    pub url: String,
    pub digest: String,
    pub extension: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl CachedImage {
    pub fn path(&self, root: &Path) -> PathBuf {
        root.join("images")
            .join(format!("{}.{}", self.digest, self.extension))
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct CachedProject {
    pub name: String,
    pub complete: bool,
    pub revision: Option<koharu_scene::Revision>,
}

pub(super) fn client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        // Each redirect is checked so a source page cannot make native requests to other hosts.
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || !is_image_url(attempt.url()) {
                attempt.error("image redirect left the Rawkuma CDN")
            } else {
                attempt.follow()
            }
        }))
        .build()?)
}

pub(super) fn is_image_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("cdn.rawkuma.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path().starts_with("/s/")
}

pub(super) fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub(super) async fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).context("invalid source cache metadata")?,
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(super) async fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let path = path.to_owned();
    let bytes = serde_json::to_vec(value)?;
    tokio::task::spawn_blocking(move || -> Result<()> {
        use std::io::Write;
        let parent = path.parent().context("cache path has no parent")?;
        std::fs::create_dir_all(parent)?;
        let mut staged = tempfile::NamedTempFile::new_in(parent)?;
        staged.write_all(&bytes)?;
        staged.as_file().sync_all()?;
        staged
            .persist(&path)
            .context("could not publish source cache metadata")?;
        Ok(())
    })
    .await??;
    Ok(())
}

pub(super) async fn download(
    client: &Client,
    root: &Path,
    url: Url,
    referer: &str,
    revalidate: bool,
) -> Result<CachedImage> {
    if !is_image_url(&url) {
        bail!("unsupported source image URL");
    }
    let meta_path = root
        .join("metadata")
        .join(format!("{}.json", digest(url.as_str().as_bytes())));
    let previous = read_json::<CachedImage>(&meta_path).await?;
    // Verify bytes, not just existence: a damaged cache must never produce a false 304 hit.
    let previous = match previous {
        Some(image)
            if image.url == url.as_str()
                && matches!(image.extension.as_str(), "jpg" | "png" | "webp") =>
        {
            match tokio::fs::read(image.path(root)).await {
                Ok(bytes) if digest(&bytes) == image.digest => Some(image),
                _ => None,
            }
        }
        _ => None,
    };
    if !revalidate && let Some(image) = &previous {
        return Ok(image.clone());
    }
    let mut request = client
        .get(url.clone())
        .header(header::REFERER, referer)
        .header(header::ACCEPT, "image/jpeg,image/png,image/webp;q=0.9");
    if let Some(image) = &previous {
        if let Some(etag) = &image.etag {
            request = request.header(header::IF_NONE_MATCH, etag);
        } else if let Some(modified) = &image.last_modified {
            request = request.header(header::IF_MODIFIED_SINCE, modified);
        }
    }
    let response = request
        .send()
        .await
        .context("could not download source image")?;
    if response.status() == StatusCode::NOT_MODIFIED {
        return previous.context("source returned 304 without a valid cached image");
    }
    if response.status() == StatusCode::FORBIDDEN {
        bail!(
            "Rawkuma refused this image (HTTP 403). Reload the chapter in the source browser and try again."
        );
    }
    let response = response.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_IMAGE_BYTES as u64)
    {
        bail!("source image exceeds 32 MiB");
    }
    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let last_modified = response
        .headers()
        .get(header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > MAX_IMAGE_BYTES {
            bail!("source image exceeds 32 MiB");
        }
        bytes.extend_from_slice(&chunk);
    }
    let format = image::guess_format(&bytes).context("source returned a non-image response")?;
    let extension = match format {
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::Png => "png",
        image::ImageFormat::WebP => "webp",
        _ => bail!("source returned an unsupported image format"),
    }
    .to_owned();
    let (width, height) =
        image::ImageReader::with_format(std::io::Cursor::new(&bytes), format).into_dimensions()?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 100_000_000 {
        bail!("source image dimensions exceed the import limit");
    }
    let image = CachedImage {
        url: url.to_string(),
        digest: digest(&bytes),
        extension,
        etag,
        last_modified,
    };
    let path = image.path(root);
    tokio::fs::create_dir_all(path.parent().unwrap()).await?;
    // The immutable content filename is only referenced after its full write completes.
    let directory = path.parent().unwrap().to_owned();
    tokio::task::spawn_blocking(move || -> Result<()> {
        use std::io::Write;
        let mut staged = tempfile::NamedTempFile::new_in(directory)?;
        staged.write_all(&bytes)?;
        staged.persist(path)?;
        Ok(())
    })
    .await??;
    write_json(&meta_path, &image).await?;
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_foreign_hosts_credentials_and_non_https_image_urls() {
        for url in [
            "http://cdn.rawkuma.com/s/book/1/1.jpg",
            "https://cdn.rawkuma.com.attacker.test/s/book/1/1.jpg",
            "https://user:pass@cdn.rawkuma.com/s/book/1/1.jpg",
            "https://127.0.0.1/s/book/1/1.jpg",
            "https://cdn.rawkuma.com:8443/s/book/1/1.jpg",
            "https://cdn.rawkuma.com/cover.jpg",
        ] {
            assert!(!is_image_url(&Url::parse(url).unwrap()), "{url}");
        }
        assert!(is_image_url(
            &Url::parse("https://cdn.rawkuma.com/s/book/16.2/1.jpg").unwrap()
        ));
    }

    #[tokio::test]
    async fn metadata_publication_replaces_previous_complete_value() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("metadata/image.json");
        write_json(
            &path,
            &CachedProject {
                name: "old".into(),
                complete: false,
                revision: None,
            },
        )
        .await
        .unwrap();
        write_json(
            &path,
            &CachedProject {
                name: "new".into(),
                complete: true,
                revision: Some(koharu_scene::Revision::new(2)),
            },
        )
        .await
        .unwrap();
        let value: CachedProject = read_json(&path).await.unwrap().unwrap();
        assert_eq!(value.name, "new");
        assert!(value.complete);
    }
}
