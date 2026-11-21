use std::future::Future;

use rsql_core::{
    ApplicationServices, Error,
    postgres::{
        ObjectStats,
        metadata_schema::{self as relations, StructureMetadata},
        metadata_views_functions::{self as routines, RoutineMetadata},
    },
};

pub use rsql_core::structure::{ObjectKind, ObjectRef};

#[derive(Debug, Clone)]
pub struct RoutineDetails {
    pub routine: RoutineMetadata,
    pub properties: ObjectStats,
    pub definition: String,
}

fn changed() -> Error {
    Error::QueryFailed("Object identity changed; refresh the catalog".into())
}

/// Reads one catalog snapshot. The existing service owns checkout, route and cancellation.
pub async fn load_structure(
    services: &ApplicationServices,
    object: &ObjectRef,
    cancelled: impl Future<Output = ()>,
) -> Result<StructureMetadata, Error> {
    services
        .run_metadata(&object.project, cancelled, async |client| {
            client
                .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
                .await
                .map_err(|e| Error::QueryFailed(e.to_string()))?;
            let relation = relations::load_relation(client, &object.schema, &object.name).await?;
            if ObjectRef::relation(&object.project, relation) != *object {
                return Err(changed());
            }
            if !relations::owns_relation(client, object.oid).await? {
                return Err(Error::QueryFailed(
                    "A complete structure editing baseline requires table ownership.".into(),
                ));
            }
            let structure = relations::load_structure(client, &object.schema, &object.name).await?;
            client
                .batch_execute("COMMIT")
                .await
                .map_err(|e| Error::QueryFailed(e.to_string()))?;
            Ok(structure)
        })
        .await
}

pub async fn load_routine(
    services: &ApplicationServices,
    object: &ObjectRef,
    cancelled: impl Future<Output = ()>,
) -> Result<RoutineDetails, Error> {
    let identity = object.routine_identity.as_ref().ok_or_else(changed)?;
    services
        .run_metadata(&object.project, cancelled, async |client| {
            client
                .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
                .await
                .map_err(|e| Error::QueryFailed(e.to_string()))?;
            let routine = routines::resolve_routine(client, identity).await?;
            if ObjectRef::routine(&object.project, routine.identity.clone()) != *object {
                return Err(changed());
            }
            let properties = routines::load_routine_info(client, identity).await?;
            let definition = routines::generate_routine_ddl(client, identity).await?;
            client
                .batch_execute("COMMIT")
                .await
                .map_err(|e| Error::QueryFailed(e.to_string()))?;
            Ok(RoutineDetails {
                routine,
                properties,
                definition,
            })
        })
        .await
}

pub mod properties;
