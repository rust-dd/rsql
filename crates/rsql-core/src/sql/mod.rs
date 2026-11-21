pub mod lexer;
pub use lexer::{Token, TokenKind, lex, scan};
use std::ops::Range;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Statement {
    pub range: Range<usize>,
    pub terminator: Option<usize>,
}
#[derive(Default)]
pub struct Segment {
    pub first: Option<Range<usize>>,
    end: usize,
    parentheses: usize,
    blocks: usize,
    body_statement_start: bool,
    prefix: RoutinePrefix,
    begin: bool,
}

#[derive(Default)]
enum RoutinePrefix {
    #[default]
    Start,
    Create,
    Or,
    Replace,
    Routine,
    Other,
}

impl RoutinePrefix {
    fn advance(&mut self, text: &str) {
        use RoutinePrefix::*;
        *self = match &*self {
            Start if text.eq_ignore_ascii_case("create") => Create,
            Create if text.eq_ignore_ascii_case("or") => Or,
            Or if text.eq_ignore_ascii_case("replace") => Replace,
            Create | Replace
                if text.eq_ignore_ascii_case("function")
                    || text.eq_ignore_ascii_case("procedure") =>
            {
                Routine
            }
            Routine => Routine,
            _ => Other,
        };
    }
}

impl Segment {
    fn range(&self) -> Option<Range<usize>> {
        self.first.as_ref().map(|r| r.start..self.end)
    }

    fn push(&mut self, t: &Token, text: &str) {
        self.first.get_or_insert_with(|| t.range.clone());
        self.end = t.range.end;
        if self.parentheses == 0 {
            if self.body_statement_start {
                self.prefix = RoutinePrefix::Start;
            }
            self.prefix.advance(text);
            // A routine END follows its last statement's semicolon (or an empty
            // BEGIN ATOMIC). An END inside a query can be a CASE end or a label.
            if self.begin && text.eq_ignore_ascii_case("atomic") {
                self.begin = false;
                self.blocks += 1;
                self.body_statement_start = true;
            } else {
                if self.body_statement_start && text.eq_ignore_ascii_case("end") {
                    self.blocks = self.blocks.saturating_sub(1);
                }
                self.begin = (matches!(self.prefix, RoutinePrefix::Routine)
                    || self.body_statement_start)
                    && text.eq_ignore_ascii_case("begin");
                self.body_statement_start = self.blocks > 0 && text == ";";
            }
        }
        match text {
            "(" => self.parentheses += 1,
            ")" => self.parentheses = self.parentheses.saturating_sub(1),
            _ => (),
        }
    }
}

pub fn segments<E>(
    sql: &str,
    check: impl FnMut() -> Result<(), E>,
    mut finish: impl FnMut(&Segment, Option<usize>),
) -> Result<(), E> {
    let mut segment = Segment::default();
    scan(sql, check, |token| {
        if token.kind == TokenKind::Comment {
            return Ok(());
        }
        let text = &sql[token.range.clone()];
        if text == ";" && segment.parentheses == 0 && segment.blocks == 0 {
            finish(&segment, Some(token.range.start));
            segment = Segment::default();
        } else {
            segment.push(&token, text)
        }
        Ok(())
    })?;
    finish(&segment, None);
    Ok(())
}

pub fn statements(sql: &str) -> Vec<Statement> {
    statements_with(sql, &mut || Ok(())).expect("uncancelled scan")
}

/// Whether the editor considers the entire draft whitespace.
pub fn is_blank(sql: &str) -> bool {
    blank_with(sql, &mut || Ok::<_, std::convert::Infallible>(())).unwrap()
}

pub fn blank_with<E>(sql: &str, check: &mut impl FnMut() -> Result<(), E>) -> Result<bool, E> {
    for (i, c) in sql.chars().enumerate() {
        if i % 4096 == 0 {
            check()?;
        }
        if !lexer::editor_whitespace(c) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn statements_with(
    sql: &str,
    check: &mut impl FnMut() -> Result<(), String>,
) -> Result<Vec<Statement>, String> {
    if blank_with(sql, check)? {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    segments(sql, check, |segment, terminator| {
        if let Some(range) = segment.range() {
            result.push(Statement { range, terminator })
        }
    })?;
    Ok(result)
}
