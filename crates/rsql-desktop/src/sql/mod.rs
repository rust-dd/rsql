mod formatter;
pub use formatter::format;
pub(crate) use rsql_core::sql::lexer::{lex_with, scan};
pub use rsql_core::sql::{Statement, Token, TokenKind, is_blank, lex, statements};
use rsql_core::sql::{blank_with, lexer, segments, statements_with};
use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetMode {
    Run,
    Script,
    Explain,
}

/// Returns the global byte range including whitespace and comments around the caret's statement.
pub fn completion_range(sql: &str, caret: usize) -> Option<Range<usize>> {
    completion_range_with(sql, caret, || Ok::<_, std::convert::Infallible>(())).unwrap()
}

pub(crate) fn completion_range_with<E>(
    sql: &str,
    caret: usize,
    check: impl FnMut() -> Result<(), E>,
) -> Result<Option<Range<usize>>, E> {
    if !sql.is_char_boundary(caret) {
        return Ok(None);
    }
    let mut from = 0;
    let mut result = None;
    segments(sql, check, |_, term| {
        let end = term.unwrap_or(sql.len());
        if (from..=end).contains(&caret) {
            result = Some(from..end)
        }
        from = end + 1;
    })?;
    Ok(result)
}

pub fn target(
    sql: &str,
    selection: Option<(usize, usize)>,
    mode: TargetMode,
) -> Result<Range<usize>, String> {
    target_with(sql, selection, mode, &mut || Ok(()))
}

/// Resolves a submitted editor target, checking ownership during the lexical scan.
pub fn execution_target(
    sql: &str,
    selection: (usize, usize),
    mode: TargetMode,
    mut check: impl FnMut() -> Result<(), String>,
) -> Result<Range<usize>, String> {
    if mode == TargetMode::Script && blank_with(sql, &mut check)? {
        return Err("There is no SQL statement to run.".into());
    }
    target_with(sql, Some(selection), mode, &mut check)
}

fn target_with(
    sql: &str,
    selection: Option<(usize, usize)>,
    mode: TargetMode,
    check: &mut impl FnMut() -> Result<(), String>,
) -> Result<Range<usize>, String> {
    if mode == TargetMode::Script {
        return Ok(0..sql.len());
    }
    if let Some((a, b)) = selection {
        let start = a.min(b);
        let selected = sql
            .get(start..a.max(b))
            .ok_or("Select a valid UTF-8 text range.")?;
        if a != b {
            let list = statements_with(selected, check)?;
            let first = list
                .first()
                .ok_or("The selection contains no SQL statement.")?;
            if mode == TargetMode::Run {
                let from = start + selected.len()
                    - selected.trim_start_matches(lexer::sql_whitespace).len();
                return Ok(from..from + selected.trim_matches(lexer::sql_whitespace).len());
            }
            if list.len() > 1 {
                return Err(format!(
                    "The selection contains {} statements. Select one statement.",
                    list.len()
                ));
            }
            return Ok(start + first.range.start..start + first.range.end);
        }
    }
    let list = statements_with(sql, check)?;
    match list.as_slice() {
        [] => Err("There is no SQL statement to run.".into()),
        [only] => Ok(only.range.clone()),
        _ => list
            .iter()
            .find(|s| {
                selection.is_some_and(|(at, _)| {
                    (s.range.start..=s.terminator.map_or(s.range.end, |n| n + 1)).contains(&at)
                })
            })
            .map(|s| s.range.clone())
            .ok_or_else(|| {
                let hint = if mode == TargetMode::Run {
                    "Place the cursor in one, select the SQL to run, or use Run script."
                } else {
                    "Place the cursor in one or select it."
                };
                format!("The editor contains several statements. {hint}")
            }),
    }
}

pub fn changes_schema(sql: &str) -> bool {
    changes_schema_with(sql, || Ok::<_, std::convert::Infallible>(())).unwrap()
}

pub(crate) fn changes_schema_with<E>(
    sql: &str,
    check: impl FnMut() -> Result<(), E>,
) -> Result<bool, E> {
    let mut changes = false;
    segments(sql, check, |segment, _| {
        if let Some(first) = &segment.first {
            changes |= [
                "CREATE", "ALTER", "DROP", "TRUNCATE", "COMMENT", "GRANT", "REVOKE", "REFRESH",
            ]
            .iter()
            .any(|word| sql[first.clone()].eq_ignore_ascii_case(word));
        }
    })?;
    Ok(changes)
}
