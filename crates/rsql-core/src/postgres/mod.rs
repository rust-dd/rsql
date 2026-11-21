//! PostgreSQL catalog, administration, and atomic edits.

pub mod ddl_generation;
pub mod extensions;
pub mod metadata_schema;
pub mod metadata_views_functions;
pub mod mutation;
pub mod provenance;
pub mod roles_schema_objects;
pub mod schema_index;
pub mod statistics_activity;
mod table_action;
mod text_param;

pub use table_action::table_action;
pub use tokio_postgres::Client;

/// Column detail info: (name, data_type, nullable, default_value)
pub type ColumnDetail = (String, String, bool, Option<String>);

/// Index info: (index_name, column_name, is_unique, is_primary)
pub type IndexDetail = (String, String, bool, bool);

/// Trigger info: (trigger_name, event, timing)
pub type TriggerDetail = (String, String, String);

/// Rule info: (rule_name, event)
pub type RuleDetail = (String, String);

/// Policy info: (policy_name, permissive, command)
pub type PolicyDetail = (String, String, String);

/// Function info: (name, return_type, arguments)
pub type FunctionInfo = (String, String, String);

/// Database stats: (stat_name, stat_value)
pub type DbStat = (String, String);

/// Constraint info: (constraint_name, constraint_type, column_name)
pub type ConstraintDetail = (String, String, String);

/// FK relation: (source_table, source_column, target_schema, target_table, target_column)
pub type ForeignKeyInfo = (String, String, String, String, String);

/// Table statistics: Vec of (key, value) pairs
pub type ObjectStats = Vec<(String, String)>;

/// FK detail: (constraint_name, source_schema, source_table, source_column, target_schema, target_table, target_column, on_update, on_delete)
pub type FKDetail = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
);
