use super::{PageError, PageService};
use crate::execution::RetainedResult;

enum Backing {
    Snapshot(RetainedResult),
    Command(String),
}

/// Original packed data for an owned result, including refused command messages.
pub struct PackedSource {
    data: Backing,
    service: PageService,
}

impl PackedSource {
    pub(crate) fn snapshot(lease: RetainedResult) -> Self {
        Self {
            service: lease.service().clone(),
            data: Backing::Snapshot(lease),
        }
    }

    pub(crate) fn command(body: String, service: PageService) -> Self {
        Self {
            data: Backing::Command(body),
            service,
        }
    }

    pub fn retained(&self) -> Option<&RetainedResult> {
        match &self.data {
            Backing::Snapshot(lease) => Some(lease),
            Backing::Command(_) => None,
        }
    }

    pub async fn fetch_packed(&self, offset: usize) -> Result<String, PageError> {
        match &self.data {
            Backing::Snapshot(lease) => lease
                .fetch_packed(offset)
                .await
                .map_err(|e| PageError::Unavailable(e.to_string())),
            Backing::Command(body) if offset == 0 => Ok(body.clone()),
            Backing::Command(_) => Err(PageError::Shape),
        }
    }

    pub(crate) async fn export_page(&self, offset: usize) -> Result<ExportPage<'_>, PageError> {
        match &self.data {
            Backing::Snapshot(lease) => lease
                .shared_page(offset)
                .await
                .map(ExportPage::Snapshot)
                .map_err(|e| PageError::Unavailable(e.to_string())),
            Backing::Command(body) if offset == 0 => Ok(ExportPage::Command(body)),
            Backing::Command(_) => Err(PageError::Shape),
        }
    }

    pub(super) fn service(&self) -> &PageService {
        &self.service
    }

    pub(super) fn is_open(&self) -> bool {
        self.retained().is_none_or(RetainedResult::is_open)
    }

    pub(super) fn can_evict(&self) -> bool {
        self.retained().is_none_or(RetainedResult::readable_now)
    }

    pub(super) async fn available(&self) -> bool {
        match self.retained() {
            Some(lease) => lease.available().await,
            None => true,
        }
    }
}

pub(crate) enum ExportPage<'a> {
    Snapshot(rsql_core::query::SharedPage),
    Command(&'a str),
}
impl AsRef<str> for ExportPage<'_> {
    fn as_ref(&self) -> &str {
        match self {
            Self::Snapshot(page) => page.as_ref(),
            Self::Command(body) => body,
        }
    }
}
