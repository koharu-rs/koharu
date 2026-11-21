//! Not upstream: the window as an error saw it, as Sentry's mobile SDKs
//! attach with `attachScreenshot`. A replay records the interface's page,
//! not what the app draws natively under it.

use std::sync::OnceLock;

use sentry::protocol::{Attachment, AttachmentType, EnvelopeItem};

type Provider = dyn Fn() -> Option<Vec<u8>> + Send + Sync;

static PROVIDER: OnceLock<Box<Provider>> = OnceLock::new();

/// Sets what takes the window's image, as JPEG bytes, for each error event;
/// `None` skips it, as on a thread the image waits on. The first call wins.
pub fn set_screenshot(provider: impl Fn() -> Option<Vec<u8>> + Send + Sync + 'static) {
    let _ = PROVIDER.set(Box::new(provider));
}

/// The window's image as an attachment Sentry shows as the event's
/// screenshot, which it does for one named `screenshot.*`.
pub(crate) fn taken() -> Option<EnvelopeItem> {
    let jpeg = PROVIDER.get().and_then(|provider| provider())?;
    Some(EnvelopeItem::Attachment(Attachment {
        buffer: jpeg,
        filename: "screenshot.jpg".to_owned(),
        content_type: Some("image/jpeg".to_owned()),
        ty: Some(AttachmentType::Attachment),
    }))
}
