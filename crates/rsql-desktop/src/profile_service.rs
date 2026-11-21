use crate::connection::{ConnectionProfile, Connections, sort_profiles};
use rsql_core::{ApplicationServices, storage::project};
use serde::Deserialize;
use std::num::NonZeroU16;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Challenge {
    pub id: String,
    pub host: String,
    pub port: NonZeroU16,
    pub fingerprint: String,
    pub previous_fingerprint: Option<String>,
}

impl Challenge {
    pub fn parse(message: &str) -> Option<Self> {
        let (_, json) = message.split_once("SSH_HOST_KEY:")?;
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        value.get("previousFingerprint")?;
        serde_json::from_value(value).ok()
    }
}

impl ConnectionProfile {
    pub fn built(mut self) -> Self {
        self.name = self.name.trim().to_owned();
        if self.name.is_empty() {
            self.name = if self.database.is_empty() {
                self.host.clone()
            } else {
                self.database.clone()
            };
        }
        if self.ssh_port.trim().is_empty() {
            self.ssh_port = "22".into();
        }
        self
    }

    pub fn validated(self) -> Result<Self, String> {
        self.check_form()?;
        Ok(self.built())
    }

    pub fn check_form(&self) -> Result<(), String> {
        for (field, value) in [
            ("connection name", &self.name),
            ("port", &self.port),
            ("database user", &self.username),
        ] {
            if value.trim().is_empty() {
                return Err(format!("Enter a {field}."));
            }
        }
        self.check_route()
    }

    pub fn check_route(&self) -> Result<(), String> {
        if self.host.trim().is_empty() {
            return Err("Enter a PostgreSQL host.".into());
        }
        if self.database.trim().is_empty() {
            return Err("Enter a database name.".into());
        }
        if self.ssh_enabled == "true" && self.ssh_host.trim().is_empty() {
            return Err("Enter an SSH host.".into());
        }
        Ok(())
    }

    pub fn safe_error(&self, message: String) -> String {
        [&self.password, &self.ssh_password]
            .into_iter()
            .filter(|secret| !secret.is_empty())
            .fold(message, |text, secret| text.replace(secret, "••••"))
    }

    pub fn clone_database(&self, database: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            database: database.into(),
            name: database.into(),
            ..self.clone()
        }
    }

    fn ssh(&self) -> Option<Vec<String>> {
        (self.ssh_enabled == "true").then(|| {
            vec![
                self.ssh_host.clone(),
                self.ssh_port.clone(),
                self.ssh_user.clone(),
                self.ssh_password.clone(),
                self.ssh_key_path.clone(),
            ]
        })
    }

    fn database_key(&self) -> [&str; 6] {
        [
            &self.username,
            &self.password,
            &self.database,
            &self.host,
            &self.port,
            &self.ssl,
        ]
    }

    pub async fn test(&self, services: &ApplicationServices) -> Result<String, String> {
        self.check_route()?;
        services
            .test_connection(self.database_key(), self.ssh())
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn save(&self, services: &ApplicationServices) -> Result<(), String> {
        let db = services.local_db.connect().map_err(|e| e.to_string())?;
        project::upsert_project(&db, &self.id, &self.details())
            .await
            .map_err(|e| e.to_string())
    }

    /// Validate the route captured by the pane against the saved row. Use the
    /// same snapshot for connection preparation so a later save cannot change
    /// the credentials between the comparison and socket creation.
    pub(crate) async fn connect_for_query(
        &self,
        services: &ApplicationServices,
        ticket: &rsql_core::services::ProjectTicket,
    ) -> Result<(), rsql_core::Error> {
        use rsql_core::Error;
        self.check_route().map_err(Error::ConnectionFailed)?;
        let db = services
            .local_db
            .connect()
            .map_err(|e| Error::DatabaseError(e.to_string()))?;
        let saved = project::select_project(&db, &self.id)
            .await
            .map_err(|e| Error::DatabaseError(e.to_string()))?
            .ok_or_else(|| Error::ProjectNotFound(self.id.clone()))?;
        let saved = Self::from_row(self.id.clone(), &saved);
        if !self.same_route(&saved) {
            return Err(Error::ConnectionFailed(
                "The saved connection has changed. Run the query again.".into(),
            ));
        }
        services
            .connect_with_ticket(ticket, Some(saved.database_key()), saved.ssh())
            .await
            .map(|_| ())
    }

    pub async fn connect_saved(&self, services: &ApplicationServices) -> Result<(), String> {
        self.check_route()?;
        services
            .connect(&self.id, None, self.ssh())
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

pub async fn profiles(services: &ApplicationServices) -> Result<Connections, String> {
    let db = services.local_db.connect().map_err(|e| e.to_string())?;
    project::select_projects(&db)
        .await
        .map(Connections::from_rows)
        .map_err(|e| e.to_string())
}

#[derive(PartialEq, Eq)]
struct Endpoint<'a>(&'a str, &'a str, &'a str, Option<(&'a str, &'a str)>);

fn endpoint(p: &ConnectionProfile) -> Endpoint<'_> {
    Endpoint(
        &p.host,
        &p.port,
        &p.username,
        (p.ssh_enabled == "true").then_some((&p.ssh_host, &p.ssh_port)),
    )
}

impl Connections {
    pub fn groups(&self) -> Vec<Vec<&ConnectionProfile>> {
        let mut groups: Vec<Vec<&ConnectionProfile>> = Vec::new();
        for profile in &self.profiles {
            if let Some(group) = groups
                .iter_mut()
                .find(|group| endpoint(group[0]) == endpoint(profile))
            {
                group.push(profile);
            } else {
                groups.push(vec![profile]);
            }
        }
        groups
    }

    pub fn put(&mut self, profile: ConnectionProfile) {
        self.profiles.retain(|p| p.id != profile.id);
        self.profiles.push(profile);
        sort_profiles(&mut self.profiles);
    }
}
