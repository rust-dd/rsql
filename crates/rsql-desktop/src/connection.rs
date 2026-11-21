use icu_collator::{
    Collator,
    options::{CollatorOptions, Strength},
};
use icu_locale_core::Locale;
use percent_encoding::percent_decode_str;
use std::collections::BTreeMap;
use url::Url;
use uuid::Uuid;

const MODES: [&str; 5] = ["disable", "prefer", "require", "verify-ca", "verify-full"];

pub fn stored_mode(value: &str) -> &str {
    match value {
        "" | "false" => "disable",
        mode if MODES.contains(&mode) => mode,
        _ => "verify-full",
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionProfile {
    pub id: String,
    pub driver: String,
    pub username: String,
    pub password: String,
    pub database: String,
    pub host: String,
    pub port: String,
    pub ssl: String,
    pub ssh_enabled: String,
    pub ssh_host: String,
    pub ssh_port: String,
    pub ssh_user: String,
    pub ssh_password: String,
    pub ssh_key_path: String,
    pub name: String,
    pub server_alias: String,
}

impl Default for ConnectionProfile {
    fn default() -> Self {
        let mut form = Self::from_row(Uuid::new_v4().to_string(), &[]);
        form.host = "localhost".into();
        form.port = "5432".into();
        form.ssl = "prefer".into();
        form
    }
}

impl ConnectionProfile {
    pub fn from_row(id: String, row: &[String]) -> Self {
        let at = |i: usize, fallback: &str| row.get(i).cloned().unwrap_or_else(|| fallback.into());
        Self {
            id,
            driver: at(0, "PGSQL"),
            username: at(1, ""),
            password: at(2, ""),
            database: at(3, ""),
            host: at(4, ""),
            port: at(5, ""),
            ssl: at(6, "false"),
            ssh_enabled: at(7, "false"),
            ssh_host: at(8, ""),
            ssh_port: at(9, "22"),
            ssh_user: at(10, ""),
            ssh_password: at(11, ""),
            ssh_key_path: at(12, ""),
            name: at(13, ""),
            server_alias: at(14, ""),
        }
    }

    fn route_fields(&self) -> [&String; 13] {
        [
            &self.driver,
            &self.username,
            &self.password,
            &self.database,
            &self.host,
            &self.port,
            &self.ssl,
            &self.ssh_enabled,
            &self.ssh_host,
            &self.ssh_port,
            &self.ssh_user,
            &self.ssh_password,
            &self.ssh_key_path,
        ]
    }

    pub fn details(&self) -> Vec<String> {
        self.route_fields()
            .into_iter()
            .chain([&self.name, &self.server_alias])
            .cloned()
            .collect()
    }

    pub fn same_route(&self, other: &Self) -> bool {
        self.id == other.id && self.route_fields() == other.route_fields()
    }

    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }
}

#[derive(Default)]
pub struct Connections {
    pub profiles: Vec<ConnectionProfile>,
    pub legacy_ids: BTreeMap<String, String>,
}
impl Connections {
    pub fn resolve_id(&self, id: &str) -> Option<&ConnectionProfile> {
        let find = |id: &str| self.profiles.iter().find(|profile| profile.id == id);
        find(id).or_else(|| find(self.legacy_ids.get(id)?))
    }

    pub fn from_rows(rows: BTreeMap<String, Vec<String>>) -> Self {
        let mut result = Self::default();
        for (id, row) in &rows {
            if let Some(old) = row
                .get(15)
                .filter(|old| !old.is_empty() && !rows.contains_key(*old))
            {
                result.legacy_ids.insert(old.clone(), id.clone());
            }
            result
                .profiles
                .push(ConnectionProfile::from_row(id.clone(), row));
        }
        sort_profiles(&mut result.profiles);
        result
    }
}

/// Base-sensitivity host-locale order, matching the original `localeCompare`; ties fall back to id.
pub fn sort_profiles(profiles: &mut [ConnectionProfile]) {
    sort_profiles_in(profiles, sys_locale::get_locale().as_deref());
}

fn sort_profiles_in(profiles: &mut [ConnectionProfile], locale: Option<&str>) {
    let locale = locale
        .and_then(|tag| tag.parse::<Locale>().ok())
        .unwrap_or(Locale::UNKNOWN);
    let mut options = CollatorOptions::default();
    options.strength = Some(Strength::Primary);
    let collator = Collator::try_new(locale.into(), options)
        .or_else(|_| Collator::try_new(Default::default(), options))
        .expect("compiled ICU root collation");
    profiles.sort_by_cached_key(|profile| {
        let mut key = Vec::new();
        let Ok(()) = collator.write_sort_key_to(profile.display_name(), &mut key);
        (key, profile.id.clone())
    });
}

pub struct ConnectionUrl {
    pub host: String,
    pub port: String,
    pub username: String,
    pub password: String,
    pub database: String,
    pub ssl: String,
}

fn decode(raw: &str, label: &str) -> Result<String, String> {
    let error = || format!("The {label} contains invalid percent-encoding.");
    for (index, _) in raw.match_indices('%') {
        if !raw
            .as_bytes()
            .get(index + 1..index + 3)
            .is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit))
        {
            return Err(error());
        }
    }
    percent_decode_str(raw)
        .decode_utf8()
        .map(|s| s.into_owned())
        .map_err(|_| error())
}

impl ConnectionUrl {
    pub fn parse(input: &str) -> Result<Self, String> {
        let raw = input.trim();
        if !raw.split_once("://").is_some_and(|(s, _)| {
            s.eq_ignore_ascii_case("postgres") || s.eq_ignore_ascii_case("postgresql")
        }) {
            return Err("A connection URL starts with postgresql:// or postgres://.".into());
        }
        let url = Url::parse(raw)
            .map_err(|_| "The connection URL is malformed. Check the host and port.")?;
        let host = decode(
            url.host_str()
                .unwrap_or("localhost")
                .trim_start_matches('[')
                .trim_end_matches(']'),
            "host",
        )?;
        if host.contains(',') {
            return Err("Multiple hosts are not supported.".into());
        }
        let port = url.port().unwrap_or(5432);
        if port == 0 {
            return Err("Port 0 is out of range.".into());
        }
        let mut ssl = None;
        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "ssl" => {
                    return Err(
                        "Use sslmode=require or sslmode=verify-full instead of \"ssl\".".into(),
                    );
                }
                "sslmode" => {
                    if ssl.is_some() {
                        return Err("Specify sslmode only once in the connection URL.".into());
                    }
                    ssl = Some(value);
                }
                _ => return Err(format!("Unsupported connection option \"{key}\".")),
            }
        }
        let ssl = ssl.as_deref().unwrap_or("prefer");
        if ssl == "allow" {
            return Err("sslmode=allow is not supported.".into());
        }
        if !MODES.contains(&ssl) {
            return Err(format!("Unknown sslmode \"{ssl}\"."));
        }
        Ok(Self {
            host,
            port: port.to_string(),
            database: decode(
                url.path().strip_prefix('/').unwrap_or(url.path()),
                "database name",
            )?,
            username: decode(url.username(), "user name")?,
            password: decode(url.password().unwrap_or(""), "password")?,
            ssl: ssl.into(),
        })
    }
}
