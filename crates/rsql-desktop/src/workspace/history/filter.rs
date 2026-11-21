use rsql_core::storage::history::HistoryFilter;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Range {
    #[default]
    Any,
    Day,
    Week,
    Month,
}

impl Range {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Any => "Any time",
            Self::Day => "Last 24 hours",
            Self::Week => "Last 7 days",
            Self::Month => "Last 30 days",
        }
    }

    fn days(self) -> Option<i64> {
        match self {
            Self::Any => None,
            Self::Day => Some(1),
            Self::Week => Some(7),
            Self::Month => Some(30),
        }
    }
}

#[derive(Default, PartialEq, Eq)]
pub(super) struct Form {
    pub text: String,
    pub project: Option<String>,
    pub database: Option<String>,
    pub success: Option<bool>,
    pub range: Range,
}

impl Form {
    pub(super) fn filter(&self, now: i64, before_id: Option<i64>) -> HistoryFilter {
        let text = self.text.trim();
        HistoryFilter {
            text: (!text.is_empty()).then(|| text.to_owned()),
            project_id: self.project.clone(),
            database: self.database.clone(),
            success: self.success,
            since: self.range.days().map(|days| now - days * 86_400_000),
            before_id,
            limit: Some(100),
        }
    }
}

pub(super) fn bytes(value: i64) -> String {
    if value < 1024 {
        format!("{value} B")
    } else if value < 1024 * 1024 {
        format!("{:.1} KB", value as f64 / 1024.)
    } else {
        format!("{:.1} MB", value as f64 / (1024. * 1024.))
    }
}
