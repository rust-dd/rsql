use super::{Token, TokenKind, lex};
use sqlformat::{Dialect, FormatOptions, Indent, QueryParams};
use sqlparser::{dialect::PostgreSqlDialect, parser::Parser};

const UNSAFE: &str = "Formatting would change SQL tokens or their meaning.";

/// Hide PostgreSQL quoting and nested comments from the layout engine. Only the
/// engine's whitespace and keyword casing can be used to construct the result.
fn placeholder(token: &Token, text: &str) -> Option<&'static str> {
    match token.kind {
        TokenKind::String => Some("'rsql'"),
        TokenKind::QuotedIdentifier => Some("\"rsql\""),
        TokenKind::Comment if text.starts_with("--") => Some("-- rsql"),
        TokenKind::Comment => Some("/* rsql */"),
        _ => None,
    }
}

fn keyword(word: &str) -> bool {
    matches!(
        word,
        "SELECT"
            | "FROM"
            | "WHERE"
            | "WITH"
            | "RECURSIVE"
            | "AS"
            | "JOIN"
            | "LEFT"
            | "RIGHT"
            | "FULL"
            | "OUTER"
            | "INNER"
            | "CROSS"
            | "ON"
            | "USING"
            | "AND"
            | "OR"
            | "NOT"
            | "EXISTS"
            | "GROUP"
            | "BY"
            | "ORDER"
            | "HAVING"
            | "LIMIT"
            | "OFFSET"
            | "ASC"
            | "DESC"
            | "NULLS"
            | "FIRST"
            | "LAST"
            | "INSERT"
            | "INTO"
            | "VALUES"
            | "CONFLICT"
            | "DO"
            | "NOTHING"
            | "UPDATE"
            | "SET"
            | "DELETE"
            | "RETURNING"
            | "CREATE"
            | "REPLACE"
            | "FUNCTION"
            | "PROCEDURE"
            | "RETURNS"
            | "LANGUAGE"
            | "CASE"
            | "WHEN"
            | "THEN"
            | "ELSE"
            | "END"
            | "DISTINCT"
            | "TRUE"
            | "FALSE"
            | "NULL"
            | "IN"
            | "IS"
            | "UNION"
            | "ALL"
            | "TABLE"
            | "ALTER"
            | "DROP"
            | "BEGIN"
            | "COMMIT"
            | "ROLLBACK"
            | "LIKE"
            | "ILIKE"
            | "BETWEEN"
            | "INTERSECT"
            | "EXCEPT"
            | "CAST"
            | "LATERAL"
            | "OVER"
            | "PARTITION"
            | "WINDOW"
            | "MATERIALIZED"
    )
}

fn sensitive_gap(left: &str, right: &str) -> bool {
    let operator = |s: &str| s.len() == 1 && "+-*/<>=~!@#%^&|?`:$.".contains(s);
    (operator(left) && operator(right))
        || (left.eq_ignore_ascii_case("u") && right == "&")
        || left == "$"
        || ((left == "." || right == ".")
            && (left.bytes().all(|b| b.is_ascii_digit())
                || right.bytes().all(|b| b.is_ascii_digit())))
        || ((left.ends_with('\'')
            || left.eq_ignore_ascii_case("e")
            || left.eq_ignore_ascii_case("b")
            || left.eq_ignore_ascii_case("x")
            || left.eq_ignore_ascii_case("n")
            || left == "&")
            && right.starts_with(['\'', '"']))
}

/// Returns a replacement only when the proposed layout preserves the original
/// token stream and the PostgreSQL parser produces an equivalent syntax tree.
pub fn format(sql: &str) -> Result<String, String> {
    let tokens = lex(sql);
    if tokens.iter().any(|t| {
        t.kind == TokenKind::Word
            && sql[t.range.clone()]
                .chars()
                .any(|c| !c.is_ascii() && super::lexer::editor_whitespace(c))
    }) {
        return Err(
            "Formatting cannot safely replace whitespace inside a PostgreSQL identifier.".into(),
        );
    }
    if tokens.iter().any(|t| !t.complete) {
        return Err(
            "Close the unfinished SQL literal, identifier or comment before formatting.".into(),
        );
    }
    let mut meaningful = tokens
        .iter()
        .filter(|token| token.kind != TokenKind::Comment)
        .peekable();
    while let Some(token) = meaningful.next() {
        let word = &sql[token.range.clone()];
        let next = meaningful.peek().map(|t| &sql[t.range.clone()]);
        if word.eq_ignore_ascii_case("select")
            && next.is_none_or(|s| s == ";" || s.eq_ignore_ascii_case("from"))
        {
            return Err("Select a projection before formatting this query.".into());
        }
        if token.kind == TokenKind::String
            && meaningful
                .peek()
                .is_some_and(|t| t.kind == TokenKind::String)
        {
            return Err("Adjacent SQL literals require their original whitespace.".into());
        }
        if word.eq_ignore_ascii_case("e") && next.is_some_and(|s| s.starts_with('\'')) {
            return Err("Ambiguous escape-string prefix.".into());
        }
    }
    let syntax = Parser::parse_sql(&PostgreSqlDialect {}, sql).map_err(|e| e.to_string())?;
    let mut masked = String::new();
    let mut end = 0;
    for t in &tokens {
        masked.push_str(&sql[end..t.range.start]);
        let raw = &sql[t.range.clone()];
        masked.push_str(placeholder(t, raw).unwrap_or(raw));
        end = t.range.end;
    }
    masked.push_str(&sql[end..]);
    let options = FormatOptions {
        dialect: Dialect::PostgreSql,
        indent: Indent::Spaces(2),
        uppercase: Some(true),
        joins_as_top_level: true,
        ..Default::default()
    };
    let layout = sqlformat::format(&masked, &QueryParams::None, &options);
    let proposed = lex(&layout);
    if proposed.len() != tokens.len() {
        return Err(UNSAFE.into());
    }
    let mut out = String::new();
    let mut end = 0;
    for (idx, (original, new)) in tokens.iter().zip(&proposed).enumerate() {
        let raw = &sql[original.range.clone()];
        let candidate = &layout[new.range.clone()];
        let gap = &layout[end..new.range.start];
        let expected = placeholder(original, raw).unwrap_or(raw);
        if original.kind != new.kind || !new.complete || !candidate.eq_ignore_ascii_case(expected) {
            return Err(UNSAFE.into());
        }
        if let Some(previous) = idx.checked_sub(1).map(|i| &tokens[i]) {
            let before = &sql[previous.range.end..original.range.start];
            if sensitive_gap(&sql[previous.range.clone()], raw)
                && (before.is_empty() != gap.is_empty()
                    || before.contains(['\n', '\r']) != gap.contains(['\n', '\r']))
            {
                return Err(UNSAFE.into());
            }
        }
        out.push_str(gap);
        if original.kind == TokenKind::Word && keyword(candidate) {
            out.push_str(candidate)
        } else {
            out.push_str(raw)
        }
        end = new.range.end;
    }
    out.push_str(&layout[end..]);
    let after = lex(&out);
    if tokens.len() != after.len()
        || !tokens.iter().zip(after).all(|(old, new)| {
            let a = &sql[old.range.clone()];
            let b = &out[new.range];
            old.kind == new.kind
                && new.complete
                && (a == b
                    || (old.kind == TokenKind::Word && keyword(b) && a.eq_ignore_ascii_case(b)))
        })
        || Parser::parse_sql(&PostgreSqlDialect {}, &out).ok().as_ref() != Some(&syntax)
    {
        return Err(UNSAFE.into());
    }
    Ok(out)
}
