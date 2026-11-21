mod build;
mod context;
mod identifiers;
mod parser;
mod scope;
mod snippets;

use rsql_core::postgres::schema_index::SchemaIndex;
use std::{collections::BTreeMap, ops::Range, sync::Arc};

pub use build::{build, suggestions};
pub use context::{MAX_COMPLETION_UTF16, analyze, fallback_context};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpectedKind {
    Table,
    View,
    Column,
    Function,
    Database,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopedRelation {
    pub name: String,
    pub schema: Option<String>,
    pub alias: Option<String>,
    /// CTEs, derived tables and table functions have no matching catalog column list.
    pub local: bool,
}

impl ScopedRelation {
    pub fn binding(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Clone, Debug, Default)]
pub struct CompletionContext {
    pub enabled: bool,
    pub range: Range<usize>,
    pub qualifier: Vec<String>,
    pub kinds: Vec<ExpectedKind>,
    pub keywords: Vec<String>,
    /// Inner scope before outer scope. SQL identifiers already have PostgreSQL case semantics.
    pub scope: Vec<ScopedRelation>,
}

#[derive(Clone, Debug, Default)]
pub struct CatalogSnapshot {
    pub default_schema: String,
    pub indexes: BTreeMap<String, Arc<SchemaIndex>>,
    /// Known schemas may be offered before their indexes have loaded.
    pub schemas: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuggestionKind {
    Column,
    Relation,
    Function,
    Schema,
    Keyword,
    Snippet,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Suggestion {
    pub kind: SuggestionKind,
    pub label: String,
    pub insert_text: String,
    pub is_snippet: bool,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    pub range: Range<usize>,
    pub sort_text: String,
    pub filter_text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub range: Range<usize>,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SymbolKind {
    Relation,
    Alias,
    Cte,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Declaration {
    pub name: String,
    pub kind: SymbolKind,
    pub range: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reference {
    pub range: Range<usize>,
    pub definition: Range<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct Analysis {
    pub completion: CompletionContext,
    pub diagnostics: Vec<Diagnostic>,
    pub declarations: Vec<Declaration>,
    pub references: Vec<Reference>,
}

pub mod worker;
