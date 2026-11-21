use super::*;
use rsql_core::postgres::metadata_schema::{ConstraintIdentity, ForeignKeyMetadata};

pub(super) struct Builder {
    graph: SchemaGraph,
    seen: HashSet<TableId>,
    failures: Vec<TableFailure>,
    outgoing: Vec<(ConstraintIdentity, ForeignKeyMetadata)>,
}

impl Builder {
    pub fn new(schema: SchemaIdentity) -> Self {
        let graph = SchemaGraph {
            schema,
            tables: Vec::new(),
            table_index: HashMap::new(),
            column_index: HashMap::new(),
            edges: Vec::new(),
            edge_index: HashMap::new(),
            constraint_names: HashMap::new(),
            connections: HashMap::new(),
            failures: Default::default(),
        };
        Self {
            graph,
            outgoing: Vec::new(),
            seen: HashSet::new(),
            failures: vec![],
        }
    }

    pub fn add(
        &mut self,
        table: TableMetadata,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        let graph = &mut self.graph;
        check()?;
        if table.relation.schema != graph.schema.name {
            return Ok(());
        }
        let id = TableId(table.relation.oid);
        if !self.seen.insert(id) {
            return Err(format!("Duplicate diagram relation OID {}", id.0));
        }
        let failure = |section, message| TableFailure {
            table: table.relation.clone(),
            section,
            message,
        };
        let constraints = match table.constraints {
            Ok(rows) => rows,
            Err(message) => {
                self.failures.push(failure(Section::Constraints, message));
                vec![]
            }
        };
        let rows = match table.columns {
            Ok(rows) => rows,
            Err(message) => {
                self.failures.push(failure(Section::Columns, message));
                return Ok(());
            }
        };
        let mut columns = Vec::with_capacity(rows.len());
        for c in rows {
            check()?;
            let column = ColumnId {
                table: id,
                number: c.number,
            };
            graph.column_index.insert(column, columns.len());
            columns.push(Column {
                id: column,
                name: c.name,
                ty: c.formatted_type,
                nullable: c.nullable,
                primary: c.primary,
                foreign: false,
            });
        }
        graph.table_index.insert(id, graph.tables.len());
        graph.tables.push(Table {
            identity: table.relation,
            columns,
        });
        graph.connections.insert(id, Connections::default());
        for constraint in constraints {
            check()?;
            if constraint.identity.relation_oid == id.0
                && let Some(key) = constraint.foreign_key
                && key.target.schema == graph.schema.name
            {
                self.outgoing.push((constraint.identity, key));
            }
        }
        Ok(())
    }

    pub fn visibility_failure(&mut self, table: RelationIdentity, message: String) {
        self.failures.push(TableFailure {
            table,
            section: Section::Visibility,
            message,
        });
    }

    pub fn finish(
        self,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<SchemaGraph, String> {
        check()?;
        let mut graph = self.graph;
        for (constraint, fk) in self.outgoing {
            let source = TableId(constraint.relation_oid);
            check()?;
            let target = TableId(fk.target.oid);
            if graph.table(target).is_none_or(|t| t.identity != fk.target) {
                continue;
            }
            let first = graph.edges.len();
            for (pair, columns) in fk.columns.into_iter().enumerate() {
                check()?;
                if columns.source.relation_oid != source.0
                    || columns.target.relation_oid != target.0
                    || !graph.visible(&columns.source)
                    || !graph.visible(&columns.target)
                {
                    continue;
                }
                let edge = Edge {
                    id: EdgeId {
                        constraint: constraint.oid,
                        pair,
                    },
                    source: (&columns.source).into(),
                    target: (&columns.target).into(),
                };
                for (column, neighbor) in [(edge.source, target), (edge.target, source)] {
                    let t = graph.table_index[&column.table];
                    graph.tables[t].columns[graph.column_index[&column]].foreign = true;
                    let connected = graph.connections.get_mut(&column.table).unwrap();
                    connected.neighbors.insert(neighbor);
                    connected.edges.insert(edge.id);
                }
                graph.edge_index.insert(edge.id, graph.edges.len());
                graph.edges.push(edge);
            }
            if graph.edges.len() > first {
                graph
                    .constraint_names
                    .insert(constraint.oid, constraint.name);
            }
        }
        graph.failures = self.failures.into();
        Ok(graph)
    }
}
