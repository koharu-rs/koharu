use std::{collections::HashSet, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use cef::*;
use tauri::Webview;
use tauri_runtime_cef::CefRuntime;
use tokio::sync::oneshot;
use url::Url;

use super::{SourcePage, cache};

wrap_string_visitor! {
    struct SourceVisitor {
        reply: Arc<parking_lot::Mutex<Option<oneshot::Sender<String>>>>,
    }

    impl CefStringVisitor {
        fn visit(&self, value: Option<&CefString>) {
            let reply = self.reply.lock().take();
            if let Some(reply) = reply {
                let _ = reply.send(value.map(ToString::to_string).unwrap_or_default());
            }
        }
    }
}

pub(super) async fn read(view: &Webview<CefRuntime>) -> Result<String> {
    let (sender, receiver) = oneshot::channel();
    let reply = Arc::new(parking_lot::Mutex::new(Some(sender)));
    // The pinned runtime's eval_with_callback unregisters its own DevTools observer
    // during delivery (observed SIGSEGV). CEF's source visitor retains its callback
    // for the delivery instead and reads the live document without that observer.
    view.with_webview(move |platform| {
        if let Some(frame) = platform.browser().main_frame() {
            let mut visitor = SourceVisitor::new(reply);
            frame.source(Some(&mut visitor));
        }
    })?;
    let html = tokio::time::timeout(Duration::from_secs(10), receiver)
        .await
        .context("the source document did not respond; wait for it to load")??;
    if html.is_empty() {
        bail!("the source document is still loading");
    }
    if html.len() > 8 * 1024 * 1024 {
        bail!("the source document exceeds 8 MiB");
    }
    Ok(html)
}

pub(super) fn parse(url: Url, html: &str) -> Result<SourcePage> {
    let document = dom_query::Document::from(html);
    let title = document.select("title").text().to_string();
    // CDN directories vary by series. Select reader pages, then validate their host;
    // a CDN cover elsewhere in the document must not become a translated page.
    let reader = document.select("section[data-image-data] img");
    let selection = if reader.is_empty() {
        document.select("img[alt*=\" - Page \"]")
    } else {
        reader
    };
    let mut seen = HashSet::new();
    let mut images = Vec::new();
    for node in selection.nodes() {
        let Some(source) = node
            .attr("data-src")
            .filter(|source| !source.is_empty())
            .or_else(|| node.attr("src"))
        else {
            continue;
        };
        let Ok(image) = url.join(&source) else {
            continue;
        };
        if cache::is_image_url(&image) && seen.insert(image.to_string()) {
            images.push(image.to_string());
        }
        if images.len() > 500 {
            bail!("a chapter cannot contain more than 500 pages");
        }
    }
    Ok(SourcePage {
        url: url.to_string(),
        title,
        images,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_any_cdn_directory_and_fractional_chapters_without_importing_covers() {
        let page = parse(
            Url::parse("https://rawkuma.net/manga/book/chapter-38.5/").unwrap(),
            r#"<img src="https://cdn.rawkuma.com/covers/book.jpg">
              <section data-image-data="1">
                <img src="https://cdn.rawkuma.com/w/book/38.5/1.webp">
                <img src="https://cdn.rawkuma.com/another-directory/book/38.5/2.png">
              </section>"#,
        )
        .unwrap();
        assert_eq!(
            page.images,
            [
                "https://cdn.rawkuma.com/w/book/38.5/1.webp",
                "https://cdn.rawkuma.com/another-directory/book/38.5/2.png",
            ]
        );
        let fallback = parse(
            Url::parse("https://rawkuma.net/manga/book/chapter-38.5/").unwrap(),
            r#"<img src="https://cdn.rawkuma.com/covers/book.jpg" alt="Book cover">
              <img src="https://cdn.rawkuma.com/w/book/38.5/1.webp" alt="Book - Chapter 38.5 - Page 1">"#,
        )
        .unwrap();
        assert_eq!(
            fallback.images,
            ["https://cdn.rawkuma.com/w/book/38.5/1.webp"]
        );
    }

    #[test]
    fn extracts_chapters_served_from_the_images_directory() {
        // Older chapters, including the observed Eiyuu Ou chapter 26, use /images/.
        let page = parse(
            Url::parse("https://rawkuma.net/manga/book/chapter-26.296992/").unwrap(),
            r#"<title>Chapter 26</title>
              <img src="https://cdn.rawkuma.com/cover.jpg">
              <section data-image-data="1">
                <img src="https://cdn.rawkuma.com/images/e/book/26/1.jpg">
                <img src="https://cdn.rawkuma.com/images/e/book/26/2.jpg">
              </section>"#,
        )
        .unwrap();
        assert_eq!(
            page.images,
            [
                "https://cdn.rawkuma.com/images/e/book/26/1.jpg",
                "https://cdn.rawkuma.com/images/e/book/26/2.jpg",
            ]
        );
    }

    #[test]
    fn extracts_ordered_lazy_pages_without_covers_duplicates_or_foreign_images() {
        let html = r#"<title>Chapter 16.2</title>
          <img src="https://rawkuma.net/cover.jpg">
          <section data-image-data="1">
            <img src="https://cdn.rawkuma.com/s/book/16.2/1.jpg">
            <img src="placeholder.png" data-src="//cdn.rawkuma.com/s/book/16.2/2.webp">
            <img src="https://cdn.rawkuma.com/s/book/16.2/1.jpg">
            <img src="https://other.test/private.jpg">
          </section>"#;
        let page = parse(
            Url::parse("https://rawkuma.net/manga/book/chapter-16.2/").unwrap(),
            html,
        )
        .unwrap();
        assert_eq!(page.title, "Chapter 16.2");
        assert_eq!(
            page.images,
            [
                "https://cdn.rawkuma.com/s/book/16.2/1.jpg",
                "https://cdn.rawkuma.com/s/book/16.2/2.webp",
            ]
        );
    }
}
