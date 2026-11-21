use super::{
    Analysis, CompletionContext, ExpectedKind,
    parser::{Budget, Projection, identifier},
    scope::Syntax,
};
use crate::sql::{Token, TokenKind, completion_range_with, lex_with};
use std::{ops::Range, sync::atomic::AtomicBool, time::Instant};
use tree_sitter::{Language, Node, Parser};

pub const MAX_COMPLETION_UTF16: usize = 200_000;

const START: &[&str] = &[
    "SELECT", "WITH", "INSERT", "UPDATE", "DELETE", "CREATE", "ALTER", "DROP", "TRUNCATE",
    "EXPLAIN", "ANALYZE", "VACUUM", "GRANT", "REVOKE", "COMMENT", "REFRESH", "BEGIN", "COMMIT",
    "ROLLBACK", "SET", "SHOW", "CALL", "COPY", "VALUES",
];

fn fallback(range: Range<usize>) -> CompletionContext {
    CompletionContext {
        enabled: true,
        range,
        keywords: START.iter().map(|s| (*s).into()).collect(),
        ..Default::default()
    }
}

pub fn analyze(
    sql: &str,
    caret: usize,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<Analysis, String> {
    let budget = Budget {
        cancelled,
        deadline,
    };
    budget.check()?;
    if !sql.is_char_boundary(caret) {
        return Err("Caret is outside the SQL text or inside a UTF-8 character.".into());
    }
    let language = Language::from(tree_sitter_sequel::LANGUAGE);
    let mut parser = Parser::new();
    let ready = parser.set_language(&language).is_ok();
    let mut projection = Projection::new(sql, &budget)?;
    let diagnostics = projection.diagnostics(&budget)?;
    let mut analysis = if ready && let Some(tree) = projection.sequel_tree(&mut parser, &budget)? {
        Syntax::read(tree.root_node(), sql, None, &budget)?.analysis
    } else {
        Analysis::default()
    };
    analysis.diagnostics = diagnostics;
    if sql.encode_utf16().take(MAX_COMPLETION_UTF16 + 1).count() <= MAX_COMPLETION_UTF16 {
        let statement = completion_range_with(sql, caret, || budget.check())?
            .ok_or("Invalid SQL statement offset.")?;
        analysis.completion = at_caret(
            &sql[statement.clone()],
            caret - statement.start,
            ready.then_some(&mut parser),
            &language,
            &budget,
        )?;
        analysis.completion.range.start += statement.start;
        analysis.completion.range.end += statement.start;
    } else {
        analysis.completion.range = caret..caret;
    }
    budget.check()?;
    Ok(analysis)
}

fn lexical_prefix<'a>(
    sql: &str,
    at: usize,
    tokens: &'a [Token],
) -> Option<(Range<usize>, &'a [Token])> {
    let before = tokens.partition_point(|t| t.range.start < at);
    let mut range = at..at;
    let mut prefix = &tokens[..before];
    if let Some((last, rest)) = prefix.split_last() {
        let end = last.range.end;
        match last.kind {
            TokenKind::Comment | TokenKind::String
                if at < end
                    || (at == end
                        && (!last.complete || sql[last.range.clone()].starts_with("--"))) =>
            {
                return None;
            }
            TokenKind::Word | TokenKind::QuotedIdentifier if at <= end => {
                range = last.range.clone();
                prefix = rest;
            }
            _ => (),
        }
    }
    Some((range, prefix))
}

pub fn fallback_context(
    sql: &str,
    caret: usize,
    cancelled: impl Fn() -> bool,
) -> Option<CompletionContext> {
    if !sql.is_char_boundary(caret)
        || sql.encode_utf16().take(MAX_COMPLETION_UTF16 + 1).count() > MAX_COMPLETION_UTF16
    {
        return None;
    }
    let tokens = lex_with(sql, || (!cancelled()).then_some(()).ok_or(())).ok()?;
    let (range, _) = lexical_prefix(sql, caret, &tokens)?;
    Some(fallback(range))
}

fn at_caret(
    sql: &str,
    at: usize,
    parser: Option<&mut Parser>,
    language: &Language,
    budget: &Budget,
) -> Result<CompletionContext, String> {
    let tokens = lex_with(sql, || budget.check())?;
    let Some((range, prefix)) = lexical_prefix(sql, at, &tokens) else {
        return Ok(CompletionContext {
            range: at..at,
            ..Default::default()
        });
    };
    let mut preceding = prefix
        .iter()
        .rev()
        .filter(|t| t.kind != TokenKind::Comment)
        .peekable();
    let mut context = fallback(range.clone());
    let Some(previous) = preceding.peek().map(|t| t.range.clone()) else {
        return Ok(context);
    };
    while let Some(dot) = preceding.next() {
        if &sql[dot.range.clone()] != "." {
            break;
        }
        let Some(id) = preceding.next() else { break };
        if !matches!(id.kind, TokenKind::Word | TokenKind::QuotedIdentifier) {
            break;
        }
        context.qualifier.push(identifier(&sql[id.range.clone()]));
    }
    context.qualifier.reverse();
    let Some(parser) = parser else {
        return Ok(context);
    };
    // The grammar can consume FROM as the identifier following a dangling dot.
    // Parsing with a complete operand also recovers aliases *after* the caret.
    const MARKER: &str = "__rsql_completion";
    let mut repaired = sql.to_owned();
    repaired.replace_range(range.clone(), &format!("{MARKER} "));
    let marker = range.start..range.start + MARKER.len();
    let Some(tree) = Projection::new(&repaired, budget)?.sequel_tree(parser, budget)? else {
        return Ok(context);
    };
    let root = tree.root_node();
    let id = root
        .descendant_for_byte_range(marker.start, marker.end)
        .filter(|n| n.kind() == "identifier" && n.byte_range() == marker);
    let state = if let Some(id) = id {
        context.kinds = expected(id);
        Some(id.parse_state())
    } else {
        // A new clause after a completed alias cannot start with an identifier.
        // The preceding token still carries the grammar's continuation state.
        root.descendant_for_byte_range(previous.start, previous.end)
            .map(|n| n.next_parse_state())
    };
    if let Some(mut allowed) = state
        .filter(|s| *s != 0)
        .and_then(|s| language.lookahead_iterator(s))
    {
        context.keywords = allowed
            .iter_names()
            .filter_map(|n| n.strip_prefix("keyword_"))
            .map(|k| k.to_ascii_uppercase())
            .collect();
    } else if id.is_some() {
        context.keywords.clear();
    }
    if id.is_none() {
        context.enabled = context.qualifier.is_empty();
        return Ok(context);
    }
    let syntax = Syntax::read(root, &repaired, Some(marker), budget)?;
    context.scope = syntax.visible(budget, context.qualifier.len() > 1)?;
    Ok(context)
}

fn expected(id: Node) -> Vec<ExpectedKind> {
    use ExpectedKind::*;
    let Some(parent) = id.parent() else {
        return vec![];
    };
    match parent.kind() {
        "field" | "column" => vec![Column, Function],
        "object_reference" => match parent.parent().map(|n| n.kind()) {
            Some("field" | "all_fields") => vec![Column, Function],
            Some("invocation") => vec![Function],
            Some(
                "relation" | "insert" | "from" | "alter_table" | "drop_table" | "drop_view"
                | "truncate_table",
            ) => vec![Table, View],
            Some("set_schema" | "drop_schema" | "alter_schema") => vec![Database],
            _ => vec![],
        },
        _ => vec![],
    }
}
