use super::{Result, TileKey};
use http_cache_rules::{
    date::parse_http_date,
    directives::{ResponseDirectives, parse_delta_seconds},
    rules::{AgeInput, current_age},
    validators::EntityTag,
};
use reqwest::header::{self, HeaderMap};
use serde::{Deserialize, Serialize};

pub(super) const METADATA_LIMIT: usize = 16 * 1024;
const FALLBACK: u64 = 7 * 24 * 3600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Metadata {
    pub key: (u8, u32, u32),
    #[serde(with = "persisted_headers")]
    fields: HeaderMap,
    pub fresh_until: u64,
    pub offline_until: Option<u64>,
    pub store: bool,
}

impl Metadata {
    pub fn response(
        key: TileKey,
        response: &HeaderMap,
        previous: Option<&Self>,
        sent: u64,
        received: u64,
    ) -> Result<Self> {
        let supplied = entity_tag(response)?;
        let mut fields = previous.map(|p| p.fields.clone()).unwrap_or_default();
        if let Some(previous) = previous {
            if let Some(new) = supplied {
                let matches = entity_tag(&previous.fields)?.is_some_and(|stored| {
                    if new.weak {
                        new.weak_eq(&stored)
                    } else {
                        new.strong_eq(&stored)
                    }
                });
                if !matches {
                    return Err("Tile validator mismatch".into());
                }
            }
            fields.remove(header::DATE);
            fields.remove(header::AGE);
        }
        let mut size = 0;
        for name in [
            header::CACHE_CONTROL,
            header::EXPIRES,
            header::AGE,
            header::DATE,
            header::ETAG,
            header::LAST_MODIFIED,
            header::VARY,
            header::PRAGMA,
        ] {
            if !response.contains_key(&name) {
                continue;
            }
            fields.remove(&name);
            for value in response.get_all(&name) {
                size += name.as_str().len() + value.as_bytes().len() + 8;
                if size > METADATA_LIMIT {
                    return Err("Tile cache headers exceed limit".into());
                }
                fields.append(&name, value.clone());
            }
        }
        let directives = ResponseDirectives::from_headers(&fields);
        let date = fields
            .get(header::DATE)
            .and_then(|v| parse_http_date(v.as_bytes(), received))
            .unwrap_or(received);
        let age = current_age(
            AgeInput {
                age_value: fields
                    .get(header::AGE)
                    .and_then(|v| parse_delta_seconds(v.as_bytes()))
                    .unwrap_or(0),
                date_value: date,
                request_time: sent,
                response_time: received,
            },
            received,
        );
        let lifetime = directives.max_age.unwrap_or_else(|| {
            fields
                .get(header::EXPIRES)
                .map(|v| {
                    parse_http_date(v.as_bytes(), received)
                        .unwrap_or(0)
                        .saturating_sub(date)
                })
                .unwrap_or(FALLBACK)
        });
        let variant = fields.get_all(header::VARY).iter().all(|v| {
            v.to_str().is_ok_and(|v| {
                v.split(',').all(|field| {
                    matches!(
                        field.trim().to_ascii_lowercase().as_str(),
                        "accept-encoding" | "accept"
                    )
                })
            })
        });
        let store = !directives.no_store && variant;
        Ok(Self {
            key: key.coordinates(),
            fields,
            fresh_until: if directives.no_cache || !store {
                0
            } else {
                received.saturating_add(lifetime.saturating_sub(age))
            },
            offline_until: (store && !directives.no_cache && !directives.must_revalidate).then(
                || {
                    directives.stale_if_error.map_or(u64::MAX, |grace| {
                        received.saturating_add(lifetime.saturating_add(grace).saturating_sub(age))
                    })
                },
            ),
            store,
        })
    }

    pub fn validators(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(tag) = self.fields.get(header::ETAG) {
            headers.insert(header::IF_NONE_MATCH, tag.clone());
        }
        if let Some(date) = self
            .fields
            .get_all(header::LAST_MODIFIED)
            .iter()
            .next_back()
        {
            headers.insert(header::IF_MODIFIED_SINCE, date.clone());
        }
        headers
    }
}

fn entity_tag(fields: &HeaderMap) -> Result<Option<EntityTag<'_>>> {
    let mut values = fields.get_all(header::ETAG).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err("Multiple tile ETag fields".into());
    }
    EntityTag::parse(value.as_bytes())
        .map(Some)
        .ok_or_else(|| "Malformed tile ETag".into())
}

mod persisted_headers {
    use super::{HeaderMap, entity_tag};
    use reqwest::header::{HeaderName, HeaderValue};
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(fields: &HeaderMap, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(
            fields
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_bytes())),
        )
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<HeaderMap, D::Error> {
        let mut fields = HeaderMap::new();
        for (name, value) in Vec::<(String, Vec<u8>)>::deserialize(deserializer)? {
            fields.append(
                HeaderName::from_bytes(name.as_bytes()).map_err(D::Error::custom)?,
                HeaderValue::from_bytes(&value).map_err(D::Error::custom)?,
            );
        }
        entity_tag(&fields).map_err(D::Error::custom)?;
        Ok(fields)
    }
}
