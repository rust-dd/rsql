use chrono::{DateTime, Local, Utc};

pub(super) fn count(text: &str) -> Option<String> {
    text.parse::<usize>()
        .ok()
        .map(crate::desktop::notifications::localized_row_count)
}

/// Mirrors the source `formatTimestamp` boundaries; unparseable text stays as received.
pub(super) fn relative(text: &str, now: DateTime<Utc>) -> Option<String> {
    let at = DateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f%#z").ok()?;
    let elapsed = now.signed_duration_since(at).num_milliseconds();
    Some(if elapsed < 60_000 {
        "just now".into()
    } else if elapsed < 3_600_000 {
        format!("{}m ago", elapsed / 60_000)
    } else if elapsed < 86_400_000 {
        format!("{}h ago", elapsed / 3_600_000)
    } else if elapsed < 2_592_000_000 {
        format!("{}d ago", elapsed / 86_400_000)
    } else {
        at.with_timezone(&Local).format("%x").to_string()
    })
}
