use super::provenance::MAX_CANDIDATE_BYTES;
use rsql_core::{
    mutation::quote_ident, postgres::provenance::ForeignReference, services::ProjectTicket,
};
use std::sync::Arc;

/// Shape evidence for the exact SELECT produced by a verified foreign-key click.
pub struct ForeignQuery {
    sql: Arc<str>,
    relation: String,
    route: ProjectTicket,
}

impl ForeignQuery {
    pub(super) fn build(
        reference: &ForeignReference,
        value: &str,
        route: ProjectTicket,
        current: impl Fn() -> bool,
    ) -> Option<Self> {
        let relation = format!(
            "{}.{}",
            quote_ident(&reference.schema),
            quote_ident(&reference.table)
        );
        let scope = if reference.partitioned { "" } else { "ONLY " };
        let mut sql = format!(
            "SELECT * FROM {scope}{relation} WHERE {} = E'",
            quote_ident(&reference.column)
        );
        for (i, ch) in value.chars().enumerate() {
            if i % 4096 == 0 && !current() {
                return None;
            }
            if matches!(ch, '\\' | '\'') {
                sql.push(ch);
            }
            sql.push(ch);
        }
        sql.push_str("' LIMIT 100");
        current().then(|| Self {
            sql: sql.into(),
            relation,
            route,
        })
    }
    pub fn sql(&self) -> &Arc<str> {
        &self.sql
    }
    pub(crate) fn relation_for(&self, sql: &str, route: &ProjectTicket) -> Option<&str> {
        (sql.len() <= MAX_CANDIDATE_BYTES
            && self.route.is_current()
            && self.route.same_generation(route)
            && self.sql.as_ref() == sql)
            .then_some(self.relation.as_str())
    }
}
