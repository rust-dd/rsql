use crate::{
    connection::Connections,
    document::{Documents, Kind, Tab},
};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, MapAccess, Visitor, value::MapAccessDeserializer},
};
use std::fmt;

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum TabType {
    Query,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedTab<Text = String> {
    pub title: String,
    pub editor_value: Text,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(rename = "type")]
    kind: TabType,
}

impl<'de> Deserialize<'de> for NamedTab {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct QueryObject;

        impl<'de> Visitor<'de> for QueryObject {
            type Value = NamedTab;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a saved query tab object")
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<NamedTab, A::Error> {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Fields {
                    title: String,
                    editor_value: String,
                    project_id: Option<String>,
                    #[serde(rename = "type")]
                    kind: String,
                }

                let fields = Fields::deserialize(MapAccessDeserializer::new(map))?;
                if fields.kind != "query" {
                    return Err(de::Error::unknown_variant(&fields.kind, &["query"]));
                }
                Ok(NamedTab {
                    title: fields.title,
                    editor_value: fields.editor_value,
                    project_id: fields.project_id,
                    kind: TabType::Query,
                })
            }
        }

        deserializer.deserialize_map(QueryObject)
    }
}

impl<Text> NamedTab<Text> {
    pub fn with_text<T>(&self, read: impl FnOnce(&Text) -> T) -> NamedTab<T> {
        NamedTab {
            title: self.title.clone(),
            editor_value: read(&self.editor_value),
            project_id: self.project_id.clone(),
            kind: TabType::Query,
        }
    }
}

pub fn capture<'a, Text>(
    documents: &'a Documents,
    mut main_text: impl FnMut(&'a Tab) -> Text,
) -> Vec<NamedTab<Text>> {
    documents
        .tabs
        .iter()
        .filter(|tab| tab.kind == Kind::Query)
        .map(|tab| NamedTab {
            title: tab.title.clone(),
            editor_value: main_text(tab),
            project_id: tab.project.clone(),
            kind: TabType::Query,
        })
        .collect()
}

pub fn encode(documents: &Documents) -> Result<String, String> {
    encode_tabs(&capture(documents, |tab| tab.main.draft.text.as_str()))
}

pub fn encode_tabs<Text: Serialize>(tabs: &[NamedTab<Text>]) -> Result<String, String> {
    serde_json::to_string(tabs).map_err(|error| error.to_string())
}

pub fn decode(source: &str) -> Result<Vec<NamedTab>, String> {
    serde_json::from_str(source).map_err(|error| format!("Invalid saved workspace: {error}"))
}

pub fn append(documents: &mut Documents, connections: &Connections, tabs: &[NamedTab]) {
    for tab in tabs {
        let project = tab
            .project_id
            .as_deref()
            .and_then(|id| connections.resolve_id(id))
            .map(|profile| profile.id.clone());
        documents.open(Kind::Query, project).main.draft.text = tab.editor_value.clone();
    }
}
