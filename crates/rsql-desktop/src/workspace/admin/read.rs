use super::*;
use rsql_core::{
    ApplicationServices, Error,
    postgres::{extensions, roles_schema_objects as roles},
};
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Clone)]
pub(super) enum Query {
    Roles,
    Enums,
    Installed,
    Available,
    Settings,
    Database(String),
    Table(String),
}

pub(super) enum Rows {
    Roles(Vec<PgRole>),
    Enums(Vec<EnumType>),
    Installed(Vec<super::extensions::Extension>),
    Available(Vec<super::extensions::Extension>),
    Settings(Vec<Setting>),
    Database(Vec<DbGrant>),
    Table(Vec<TableGrant>),
}

pub(super) async fn load(
    services: Arc<ApplicationServices>,
    capture: catalog::Capture,
    query: Query,
    mut request: watch::Receiver<()>,
    mut stop: watch::Receiver<bool>,
) -> Result<Rows, String> {
    let mut generation = capture.generation.clone();
    let cancelled = async {
        tokio::select! {
            _ = request.changed() => (),
            _ = generation.changed() => (),
            _ = capture.route.retired() => (),
            _ = stop.wait_for(|v| *v) => (),
        }
    };
    services
        .run_metadata(&capture.project, cancelled, async |client| {
            if !capture.route.is_current() || !matches!(capture.generation.has_changed(), Ok(false))
            {
                return Err(Error::ConnectionFailed("Connection changed".into()));
            }
            Ok(match query {
                Query::Roles => Rows::Roles(roles::load_roles(client).await?),
                Query::Installed => Rows::Installed(
                    extensions::load_extensions(client)
                        .await?
                        .into_iter()
                        .map(super::extensions::Extension::installed)
                        .collect::<Result<_, _>>()?,
                ),
                Query::Available => Rows::Available(
                    extensions::load_available_extensions(client)
                        .await?
                        .into_iter()
                        .map(super::extensions::Extension::available)
                        .collect::<Result<_, _>>()?,
                ),
                Query::Enums => Rows::Enums(extensions::load_enum_type_details(client).await?),
                Query::Settings => Rows::Settings(
                    extensions::load_pg_settings(client)
                        .await?
                        .into_iter()
                        .map(Setting::try_from)
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                Query::Database(name) => {
                    Rows::Database(roles::load_database_grants(client, &name).await?)
                }
                Query::Table(name) => Rows::Table(roles::load_table_grants(client, &name).await?),
            })
        })
        .await
        .map_err(|e| e.to_string())
}
