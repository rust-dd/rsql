use crate::{
    Error,
    error::query_failed,
    structure::{ForeignTarget, StructurePlan},
};
use tokio_postgres::Transaction;

pub(super) struct Reference {
    name: String,
    relation: u32,
    columns: Vec<i16>,
}

pub(super) async fn resolve(
    tx: &Transaction<'_>,
    plan: &StructurePlan,
) -> Result<Vec<Reference>, Error> {
    let mut references = Vec::new();
    for fk in &plan.foreign_keys {
        let ForeignTarget::Relation {
            schema,
            table,
            columns,
        } = &fk.target
        else {
            continue;
        };
        let rows = tx.query(
            "SELECT c.oid, a.attnum,n.oid FROM pg_catalog.pg_namespace n
             JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
             JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped
             JOIN pg_catalog.unnest($3::text[]) WITH ORDINALITY k(name, pos) ON k.name=a.attname
             WHERE n.nspname=$1 AND c.relname=$2 ORDER BY k.pos", &[schema,table,columns],
        ).await.map_err(query_failed)?;
        if rows.len() != columns.len() || rows.is_empty() {
            return Err(Error::QueryFailed(format!(
                "Refresh the referenced columns for foreign key {:?}.",
                fk.key.name
            )));
        }
        let columns = rows.iter().map(|r| r.get::<_, i16>(1)).collect::<Vec<_>>();
        if let Some((ns, oid, keys)) = plan.foreign_identities.get(&fk.key.name)
            && (*ns != rows[0].get::<_, u32>(2)
                || *oid != rows[0].get::<_, u32>(0)
                || keys != &columns)
        {
            return Err(Error::QueryFailed(format!(
                "The selected target of foreign key {:?} changed. Select its table and columns again.",
                fk.key.name
            )));
        }
        references.push(Reference {
            name: fk.key.name.clone(),
            relation: rows[0].get(0),
            columns,
        });
    }
    Ok(references)
}

pub(super) async fn verify(
    tx: &Transaction<'_>,
    source: u32,
    references: &[Reference],
) -> Result<(), Error> {
    for reference in references {
        let row = tx.query_opt("SELECT confrelid,confkey FROM pg_catalog.pg_constraint WHERE conrelid=$1 AND conname=$2 AND contype='f'", &[&source,&reference.name]).await.map_err(query_failed)?;
        if !row.is_some_and(|r| {
            r.get::<_, u32>(0) == reference.relation && r.get::<_, Vec<i16>>(1) == reference.columns
        }) {
            return Err(Error::QueryFailed(format!(
                "The referenced table or columns changed for foreign key {:?}. Refresh and review again.",
                reference.name
            )));
        }
    }
    Ok(())
}
