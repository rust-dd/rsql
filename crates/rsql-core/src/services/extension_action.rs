use super::{ProjectTicket, object_action::ActionOutcome, write_session::run_write_session};
use crate::{ApplicationServices, error::query_failed, mutation::quote_ident};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExtensionAction {
    Install,
    Update,
    Drop,
}

/// One immutable statement reviewed for a specific connection generation.
pub struct ExtensionReview {
    route: ProjectTicket,
    sql: String,
}

impl ExtensionAction {
    fn statement(self, name: &str) -> String {
        let identifier = quote_ident(name);
        match self {
            Self::Install => format!("CREATE EXTENSION IF NOT EXISTS {identifier};"),
            Self::Update => format!("ALTER EXTENSION {identifier} UPDATE;"),
            Self::Drop => format!("DROP EXTENSION IF EXISTS {identifier} CASCADE;"),
        }
    }

    pub fn review(self, route: &ProjectTicket, name: &str) -> ExtensionReview {
        ExtensionReview {
            route: route.clone(),
            sql: self.statement(name),
        }
    }
}

impl ExtensionReview {
    pub fn sql(&self) -> &str {
        &self.sql
    }
}

impl ApplicationServices {
    pub async fn run_extension_action(
        &self,
        id: &str,
        review: ExtensionReview,
        closed: impl Future<Output = ()>,
    ) -> ActionOutcome {
        run_write_session(
            self,
            &review.route,
            id,
            None,
            async |tx, notices| {
                let scope = notices.begin()?;
                tx.batch_execute(&review.sql).await.map_err(query_failed)?;
                Ok(scope.finish())
            },
            closed,
        )
        .await
        .into()
    }
}
