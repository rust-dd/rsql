use rsql_core::{Error, postgres::extensions::EnumType};
use tokio::sync::watch;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Section {
    pub pending: Option<(Uuid, watch::Sender<()>)>,
    pub error: Option<String>,
    pub loaded: bool,
}

impl Section {
    pub fn ready(&self) -> bool {
        self.loaded && self.error.is_none() && self.pending.is_none()
    }

    pub fn begin(&mut self) -> (Uuid, watch::Receiver<()>) {
        let id = Uuid::new_v4();
        let (cancel, receiver) = watch::channel(());
        self.pending = Some((id, cancel));
        self.error = None;
        (id, receiver)
    }

    pub fn finish(&mut self, id: Uuid) -> bool {
        if self
            .pending
            .as_ref()
            .is_some_and(|(current, _)| *current == id)
        {
            self.pending = None;
            self.loaded = true;
            true
        } else {
            false
        }
    }

    pub fn suspend(&mut self) {
        if self.pending.take().is_some() {
            self.loaded = false;
        }
    }
}

pub(super) struct Setting {
    pub name: String,
    pub value: String,
    pub unit: String,
    pub category: String,
    pub description: String,
    pub context: String,
    pub source: String,
    pub boot: String,
    pub reset: String,
}

impl TryFrom<Vec<String>> for Setting {
    type Error = Error;
    fn try_from(row: Vec<String>) -> Result<Self, Error> {
        let [
            name,
            value,
            unit,
            category,
            description,
            context,
            source,
            boot,
            reset,
        ] = <[String; 9]>::try_from(row)
            .map_err(|_| Error::QueryFailed("Incomplete PostgreSQL setting".into()))?;
        Ok(Self {
            name,
            value,
            unit,
            category,
            description,
            context,
            source,
            boot,
            reset,
        })
    }
}

impl Setting {
    pub fn modified(&self) -> bool {
        self.value != self.boot
    }
}

#[derive(Default)]
pub(super) struct Filters {
    pub category: Option<String>,
    pub context: Option<String>,
    pub text: String,
}

impl Filters {
    pub fn setting(&self, s: &Setting) -> bool {
        self.category.as_ref().is_none_or(|c| c == &s.category)
            && self.context.as_ref().is_none_or(|c| c == &s.context)
            && (s.name.to_lowercase().contains(&self.text)
                || s.description.to_lowercase().contains(&self.text))
    }

    pub fn enum_type(&self, e: &EnumType) -> bool {
        std::iter::once(&e.schema)
            .chain(std::iter::once(&e.name))
            .chain(&e.labels)
            .any(|v| v.to_lowercase().contains(&self.text))
    }

    pub fn applied(&self) -> bool {
        self.category.is_some() || self.context.is_some() || !self.text.is_empty()
    }
}
