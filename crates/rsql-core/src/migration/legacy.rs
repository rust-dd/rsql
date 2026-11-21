use serde::{Deserialize, Deserializer, de};
use serde_json::value::RawValue;
use std::{collections::HashSet, fmt};

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("Invalid legacy rsql-tabs data")]
    InvalidLegacyTabs,
    #[error("Unknown legacy rsql-tabs version {0}")]
    UnsupportedLegacyVersion(u64),
    #[error("Legacy tab {tab_number} refers to a connection missing from the snapshot")]
    UnresolvedProject { tab_number: usize },
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(default)]
    version: u64,
    #[serde(borrow)]
    state: &'a RawValue,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct State<'a> {
    tabs: LegacyTabs,
    #[serde(borrow)]
    selected_tab_index: Option<&'a RawValue>,
    #[serde(borrow)]
    selected_tab_id: Option<&'a RawValue>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LegacyKind {
    Query,
    Terminal,
    Monitor,
    Erd,
    Notify,
    Roles,
    SchemaDiff,
    Extensions,
    Enums,
    PgSettings,
}

impl LegacyKind {
    fn deserialize_name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Self::deserialize(de::value::StringDeserializer::new(name))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabFields {
    pub id: String,
    #[serde(rename = "type", deserialize_with = "LegacyKind::deserialize_name")]
    pub kind: LegacyKind,
    pub title: String,
    pub project_id: Option<String>,
    pub schema: Option<String>,
    #[serde(default)]
    pub editor_value: String,
    pub query_timeout: Option<u64>,
    #[serde(default)]
    pub is_split: bool,
    #[serde(default)]
    pub split_editor_value: String,
}

pub struct LegacyTab {
    pub fields: TabFields,
    pub number: usize,
}

#[derive(Default)]
struct LegacyTabs {
    valid: Vec<LegacyTab>,
    discarded: usize,
}

impl<'de> Deserialize<'de> for LegacyTabs {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TabList;

        impl<'de> de::Visitor<'de> for TabList {
            type Value = LegacyTabs;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a legacy tab list")
            }

            fn visit_seq<A: de::SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<LegacyTabs, A::Error> {
                let mut tabs = LegacyTabs::default();
                let mut seen = HashSet::new();
                let mut number = 0;
                while let Some(raw) = sequence.next_element::<&RawValue>()? {
                    number += 1;
                    let Some(tab) = parse_tab(raw.get(), number) else {
                        tabs.discarded += 1;
                        continue;
                    };
                    if seen.insert(tab.fields.id.clone()) {
                        tabs.valid.push(tab);
                    } else {
                        tabs.discarded += 1;
                    }
                }
                Ok(tabs)
            }
        }

        deserializer.deserialize_seq(TabList)
    }
}

fn parse_tab(raw: &str, number: usize) -> Option<LegacyTab> {
    if !raw.starts_with('{') {
        return None;
    }
    let fields = serde_json::from_str::<TabFields>(raw).ok()?;
    if fields.id.is_empty() {
        return None;
    }
    if matches!(fields.kind, LegacyKind::Erd) && fields.schema.is_none() {
        return None;
    }
    Some(LegacyTab { fields, number })
}

fn decode_object<'a, T: Deserialize<'a>>(raw: &'a str) -> Result<T, MigrationError> {
    if !raw.trim_start().starts_with('{') {
        return Err(MigrationError::InvalidLegacyTabs);
    }
    serde_json::from_str(raw).map_err(|_| MigrationError::InvalidLegacyTabs)
}

impl State<'_> {
    fn selected(&self) -> usize {
        let last = self.tabs.valid.len().saturating_sub(1);
        let selected_id = self
            .selected_tab_id
            .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok());
        self.tabs
            .valid
            .iter()
            .position(|tab| Some(tab.fields.id.as_str()) == selected_id.as_deref())
            .unwrap_or_else(|| {
                let index = self
                    .selected_tab_index
                    .and_then(|raw| serde_json::from_str::<f64>(raw.get()).ok())
                    .filter(|index| index.is_finite() && index.fract() == 0.0)
                    .unwrap_or_default();
                (index as usize).min(last)
            })
    }
}

pub struct DecodedTabs {
    pub tabs: Vec<LegacyTab>,
    pub selected: usize,
    pub discarded: usize,
}

pub fn decode_legacy_tabs(raw: &str) -> Result<DecodedTabs, MigrationError> {
    let envelope = decode_object::<Envelope<'_>>(raw)?;
    if envelope.version != 0 {
        return Err(MigrationError::UnsupportedLegacyVersion(envelope.version));
    }
    let state = decode_object::<State<'_>>(envelope.state.get())?;
    Ok(DecodedTabs {
        selected: state.selected(),
        tabs: state.tabs.valid,
        discarded: state.tabs.discarded,
    })
}
