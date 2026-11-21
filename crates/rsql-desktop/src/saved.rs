use crate::connection::{ConnectionProfile, Connections};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SavedQueryKey {
    pub project: String,
    pub database: String,
    pub driver: String,
    pub title: String,
}

impl SavedQueryKey {
    pub fn parse(id: &str) -> Option<Self> {
        let mut fields = id.splitn(4, ':');
        Some(Self {
            project: fields.next()?.into(),
            database: fields.next()?.into(),
            driver: fields.next()?.into(),
            title: fields.next()?.into(),
        })
    }

    pub fn encode(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.project, self.database, self.driver, self.title
        )
    }

    pub fn connection<'a>(&self, connections: &'a Connections) -> Option<&'a ConnectionProfile> {
        connections.resolve_id(&self.project)
    }
}

pub struct SavedQuery {
    pub key: SavedQueryKey,
    pub sql: String,
}

impl SavedQuery {
    pub fn decode(id: &str, sql: String) -> Option<Self> {
        Some(Self {
            key: SavedQueryKey::parse(id)?,
            sql,
        })
    }
}
