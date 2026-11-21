use super::{PropertyText, policy_line, trigger_line};
use rsql_core::postgres::metadata_schema::{AttributeIdentity, ConstraintMetadata};

pub enum DetailKind {
    Primary,
    Foreign,
    Unique,
    Check,
    Constraint,
    Trigger,
    Rule,
    Policy,
}

pub struct DetailRow {
    pub kind: DetailKind,
    pub name: String,
    pub badge: Option<String>,
    pub value: Option<String>,
    pub exact: PropertyText,
}

impl From<ConstraintMetadata> for DetailRow {
    fn from(constraint: ConstraintMetadata) -> Self {
        let (kind, badge) = match constraint.kind.as_str() {
            "p" => (DetailKind::Primary, "PRIMARY KEY"),
            "f" => (DetailKind::Foreign, "FOREIGN KEY"),
            "u" => (DetailKind::Unique, "UNIQUE"),
            "c" => (DetailKind::Check, "CHECK"),
            "x" => (DetailKind::Constraint, "EXCLUDE"),
            "n" => (DetailKind::Constraint, "NOT NULL"),
            _ => (DetailKind::Constraint, constraint.kind.as_str()),
        };
        let name = constraint.identity.name;
        let exact = format!("{name} · {}", constraint.definition).into();
        Self {
            kind,
            name,
            badge: Some(badge.to_owned()),
            value: constraint_column_preview(&constraint.columns),
            exact,
        }
    }
}

fn constraint_column_preview(columns: &[AttributeIdentity]) -> Option<String> {
    if columns.is_empty() {
        return None;
    }
    let mut preview = String::from("(");
    for (index, column) in columns.iter().enumerate() {
        let separator = if index == 0 { "" } else { ", " };
        let reserved = if index + 1 == columns.len() {
            ")".len()
        } else {
            ", …)".len()
        };
        if preview.len() + separator.len() + column.name.len() + reserved > 4096 {
            preview.push_str(separator);
            preview.push('…');
            break;
        }
        preview.push_str(separator);
        preview.push_str(&column.name);
    }
    preview.push(')');
    Some(preview)
}

impl DetailRow {
    pub(super) fn trigger(row: (String, String, String)) -> Self {
        let exact = trigger_line(row.clone()).into();
        let (name, event, timing) = row;
        Self {
            kind: DetailKind::Trigger,
            name,
            badge: Some(format!("{timing} {event}")),
            value: None,
            exact,
        }
    }

    pub(super) fn policy(row: (String, String, String)) -> Self {
        let exact = policy_line(row.clone()).into();
        let (name, mode, command) = row;
        Self {
            kind: DetailKind::Policy,
            name,
            badge: Some(format!("{mode} {command}")),
            value: None,
            exact,
        }
    }

    pub(super) fn rule((name, event): (String, String)) -> Self {
        let value = super::structure::rule_event(&event).to_owned();
        let exact = format!("{name} · {value}").into();
        Self {
            kind: DetailKind::Rule,
            name,
            badge: None,
            value: Some(value),
            exact,
        }
    }
}
