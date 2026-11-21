use super::ValidationError;
use crate::sql::{TokenKind, lex, statements};
use pg_query::{
    NodeEnum, ParseResult, ParserOptions,
    protobuf::{AlterTableCmd, AlterTableType},
};
use std::borrow::Cow;

fn parse(sql: &str, label: &str) -> Result<ParseResult, ValidationError> {
    pg_query::parse(sql, ParserOptions::DEFAULT)
        .map_err(|err| ValidationError(format!("Invalid PostgreSQL {label}: {err}")))
}

fn boundary<'a>(input: &'a str, label: &str) -> Result<Cow<'a, str>, ValidationError> {
    let tokens = lex(input);
    if input.contains('\0')
        || tokens
            .iter()
            .any(|t| !t.complete || &input[t.range.clone()] == ";")
    {
        return Err(ValidationError(format!("Invalid boundary in {label}.")));
    }
    if tokens.iter().all(|t| t.kind == TokenKind::Comment) {
        return Err(ValidationError(format!("Enter a {label}.")));
    }
    if tokens.last().is_some_and(|t| {
        t.kind == TokenKind::Comment
            && input[t.range.clone()].starts_with("--")
            && !input[t.range.end..].contains(['\r', '\n'])
    }) {
        // Preserve the supplied comment without letting it absorb a generated delimiter.
        return Ok(Cow::Owned(format!("{input}\n")));
    }
    Ok(Cow::Borrowed(input))
}

fn only_alter(tree: &ParseResult) -> Option<&AlterTableCmd> {
    let [stmt] = tree.protobuf.stmts.as_slice() else {
        return None;
    };
    let NodeEnum::AlterTableStmt(alter) = stmt.stmt.as_ref()?.node.as_ref()? else {
        return None;
    };
    let [command] = alter.cmds.as_slice() else {
        return None;
    };
    let NodeEnum::AlterTableCmd(command) = command.node.as_ref()? else {
        return None;
    };
    Some(command)
}

pub(super) fn data_type(input: &str) -> Result<Cow<'_, str>, ValidationError> {
    let raw = boundary(input, "data type")?;
    let tree = parse(
        &format!("ALTER TABLE t ALTER COLUMN c TYPE {raw};"),
        "data type",
    )?;
    if let Some(cmd) = only_alter(&tree)
        && cmd.subtype() == AlterTableType::AtAlterColumnType
        && let Some(NodeEnum::ColumnDef(col)) = cmd.def.as_ref().and_then(|n| n.node.as_ref())
        && col.type_name.is_some()
        && col.coll_clause.is_none()
        && col.raw_default.is_none()
    {
        Ok(raw)
    } else {
        Err(ValidationError(
            "Enter exactly one PostgreSQL data type without COLLATE or USING clauses.".into(),
        ))
    }
}

pub(super) fn default(input: &str) -> Result<Cow<'_, str>, ValidationError> {
    let raw = boundary(input, "default expression")?;
    let tree = parse(
        &format!("ALTER TABLE t ALTER COLUMN c SET DEFAULT {raw};"),
        "default expression",
    )?;
    if only_alter(&tree)
        .is_some_and(|cmd| cmd.subtype() == AlterTableType::AtColumnDefault && cmd.def.is_some())
    {
        Ok(raw)
    } else {
        Err(ValidationError(
            "Enter exactly one PostgreSQL default expression.".into(),
        ))
    }
}

pub(super) fn statement(sql: &str) -> Result<(), ValidationError> {
    if matches!(statements(sql).as_slice(), [s] if s.terminator == Some(sql.len() - 1))
        && parse(sql, "structure statement")?.protobuf.stmts.len() == 1
    {
        Ok(())
    } else {
        Err(ValidationError(
            "Expected one complete structure statement.".into(),
        ))
    }
}

pub(super) fn equivalent(catalog: &str, plain: &str) -> bool {
    let canonical = |sql| {
        pg_query::parse(sql, ParserOptions::DEFAULT)
            .and_then(|p| p.deparse(Default::default()))
            .ok()
    };
    canonical(catalog)
        .zip(canonical(plain))
        .is_some_and(|(a, b)| a == b)
}
