use std::sync::Arc;

pub struct PropertyText {
    pub full: Arc<str>,
    pub preview: Arc<str>,
}

impl From<String> for PropertyText {
    fn from(text: String) -> Self {
        let full = Arc::<str>::from(text);
        let end = full.floor_char_boundary(full.len().min(4096));
        let end = full[..end]
            .match_indices('\n')
            .nth(23)
            .map_or(end, |(i, _)| i);
        let preview = if end == full.len() {
            full.clone()
        } else {
            format!("{}\n…", &full[..end]).into()
        };
        Self { full, preview }
    }
}

impl PropertyText {
    pub fn abbreviated(&self) -> bool {
        !Arc::ptr_eq(&self.full, &self.preview)
    }
}
