use std::sync::Arc;

use data_url::DataUrl;
use gpui::{Image, ImageFormat};

const BULLETS: [&str; 5] = ["•", "◦", "▪", "‣", "⁃"];

/// Returns an ordered-list item's ordinal, defaulting omitted starts to one.
pub(super) fn ordered_list_ordinal(start: Option<u32>, ix: usize) -> u32 {
    start
        .unwrap_or(1)
        .saturating_add(u32::try_from(ix).unwrap_or(u32::MAX))
}

/// Returns the prefix for a list item.
pub(super) fn list_item_prefix(
    ix: usize,
    start: Option<u32>,
    ordered: bool,
    depth: usize,
) -> String {
    if ordered {
        let ordinal = ordered_list_ordinal(start, ix);
        if depth == 0 || start == Some(0) {
            return format!("{ordinal}. ");
        }

        let first_letter = if depth == 1 { b'A' } else { b'a' };
        let mut remaining = ordinal;
        let mut letters = Vec::new();
        while remaining > 0 {
            remaining -= 1;
            letters.push((first_letter + (remaining % 26) as u8) as char);
            remaining /= 26;
        }
        let label: String = letters.into_iter().rev().collect();
        return format!("{label}. ");
    } else {
        let depth = depth.min(BULLETS.len() - 1);
        let bullet = BULLETS[depth];
        return format!("{} ", bullet);
    }
}

/// Decodes a `data:` URL whose mime type names an image format GPUI can
/// decode. Anything else (another scheme, a non-image body, malformed
/// base64) yields `None`, and the caller keeps the URL URI-backed so GPUI's
/// own loader reports the failure.
pub(super) fn data_url_image(url: &str) -> Option<Arc<Image>> {
    let data_url = DataUrl::process(url).ok()?;
    let mime = data_url.mime_type();
    let format = ImageFormat::from_mime_type(&format!("{}/{}", mime.type_, mime.subtype))?;
    let (bytes, _fragment) = data_url.decode_to_vec().ok()?;
    Some(Arc::new(Image::from_bytes(format, bytes)))
}
