use sqlparser::{ast::*, dialect::PostgreSqlDialect, parser::Parser};

use super::foreign_query::ForeignQuery;
use rsql_core::services::ProjectTicket;

pub(super) const MAX_CANDIDATE_BYTES: usize = 65_536;

pub fn candidate_relation(
    sql: &str,
    generated: Option<&ForeignQuery>,
    route: &ProjectTicket,
) -> Option<String> {
    generated
        .and_then(|query| query.relation_for(sql, route))
        .map(str::to_owned)
        .or_else(|| single_relation(sql))
}

fn scalar(expr: &Expr) -> bool {
    match expr {
        Expr::Identifier(_) | Expr::CompoundIdentifier(_) | Expr::Value(_) => true,
        Expr::Nested(e) | Expr::IsNull(e) | Expr::IsNotNull(e) | Expr::UnaryOp { expr: e, .. } => {
            scalar(e)
        }
        Expr::BinaryOp { left, right, .. } => scalar(left) && scalar(right),
        _ => false,
    }
}

/// Bound parser work per candidate. All other SQL still uses ordinary execution.
pub fn single_relation(sql: &str) -> Option<String> {
    if sql.len() > MAX_CANDIDATE_BYTES {
        return None;
    }
    let statements = Parser::parse_sql(&PostgreSqlDialect {}, sql).ok()?;
    let [Statement::Query(q)] = statements.as_slice() else {
        return None;
    };
    let SetExpr::Select(s) = q.body.as_ref() else {
        return None;
    };
    let [from] = s.from.as_slice() else {
        return None;
    };
    let TableFactor::Table {
        name,
        args: None,
        alias,
        ..
    } = &from.relation
    else {
        return None;
    };
    if q.with.is_some() || !q.locks.is_empty() || q.fetch.is_some() || !q.pipe_operators.is_empty()
        || s.into.is_some() || s.distinct.is_some() || s.having.is_some() || !from.joins.is_empty()
        || !matches!(&s.group_by,GroupByExpr::Expressions(exprs,_) if exprs.is_empty())
        || !s.named_window.is_empty() || !s.selection.as_ref().is_none_or(scalar)
        || alias.as_ref().is_some_and(|a| !a.columns.is_empty())
        || !s.projection.iter().all(|p| match p {
            SelectItem::UnnamedExpr(e) | SelectItem::ExprWithAlias{expr:e,..}=>scalar(e),
            SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_,_)=>true,
        })
        || q.order_by.as_ref().is_some_and(|o| !matches!(&o.kind,OrderByKind::Expressions(es) if es.iter().all(|e|scalar(&e.expr))))
        || q.limit_clause.as_ref().is_some_and(|l| !matches!(l,LimitClause::LimitOffset{limit,offset,limit_by} if limit_by.is_empty() && limit.as_ref().is_none_or(scalar) && offset.as_ref().is_none_or(|o|scalar(&o.value))))
        || !(1..=2).contains(&name.0.len()) {return None}
    Some(name.to_string())
}
