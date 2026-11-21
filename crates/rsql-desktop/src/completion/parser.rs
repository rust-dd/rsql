use super::Diagnostic;
use crate::sql::{TokenKind, scan};
use std::{
    ops::{ControlFlow, Range},
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
use tree_sitter::{ParseOptions, Parser, Tree};

pub(super) struct Budget<'a> {
    pub cancelled: &'a AtomicBool,
    pub deadline: Instant,
}

impl Budget<'_> {
    pub fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err("SQL analysis cancelled".into())
        } else if Instant::now() >= self.deadline {
            Err("SQL analysis deadline exceeded".into())
        } else {
            Ok(())
        }
    }
}

const CHUNK: usize = 8192;

pub(super) struct Projection {
    bytes: Vec<u8>,
    unclosed_comment: Option<Range<usize>>,
    identifiers: Vec<(Range<usize>, u8)>,
}

impl Projection {
    pub fn new(sql: &str, budget: &Budget) -> Result<Self, String> {
        let mut bytes = Vec::with_capacity(sql.len());
        for chunk in sql.as_bytes().chunks(CHUNK) {
            budget.check()?;
            bytes.extend_from_slice(chunk);
        }
        let mut unclosed_comment = None;
        let mut identifiers = Vec::new();
        scan(
            sql,
            || budget.check(),
            |token| {
                let range = token.range;
                match token.kind {
                    TokenKind::Comment => {
                        if !token.complete {
                            unclosed_comment = Some(range.start..range.start + 2);
                        }
                        // Both grammars end a nested block comment at the inner terminator.
                        for chunk in bytes[range].chunks_mut(CHUNK) {
                            budget.check()?;
                            for c in chunk {
                                if !matches!(c, b'\n' | b'\r') {
                                    *c = b' ';
                                }
                            }
                        }
                    }
                    TokenKind::QuotedIdentifier if token.complete => {
                        identifiers.push((range.start + 1..range.end - 1, b'x'))
                    }
                    TokenKind::Word => {
                        for chunk in bytes[range.clone()].chunks(CHUNK) {
                            budget.check()?;
                            if !chunk.is_ascii() || chunk.contains(&b'$') {
                                identifiers.push((range, b'_'));
                                break;
                            }
                        }
                    }
                    _ => (),
                }
                Ok(())
            },
        )?;
        Ok(Self {
            bytes,
            identifiers,
            unclosed_comment,
        })
    }

    pub fn diagnostics(&self, budget: &Budget) -> Result<Vec<Diagnostic>, String> {
        let mut diagnostics: Vec<_> = self
            .unclosed_comment
            .iter()
            .map(|range| Diagnostic {
                range: range.clone(),
                message: "Unterminated block comment".into(),
            })
            .collect();
        let mut parser = Parser::new();
        let language = tree_sitter_postgres::LANGUAGE.into();
        if parser.set_language(&language).is_err() {
            return Ok(diagnostics);
        }
        let Some(tree) = parse(&mut parser, &self.bytes, budget)? else {
            return Ok(diagnostics);
        };
        let mut pending = vec![tree.root_node()];
        while let Some(node) = pending.pop() {
            budget.check()?;
            if node.is_error() || node.is_missing() {
                diagnostics.push(Diagnostic {
                    range: node.byte_range(),
                    message: if node.is_missing() {
                        format!("Expected {}", node.kind())
                    } else {
                        "Unexpected SQL syntax".into()
                    },
                });
            } else if node.has_error() {
                let start = pending.len();
                for child in node.children(&mut node.walk()) {
                    budget.check()?;
                    if child.has_error() {
                        pending.push(child);
                    }
                }
                pending[start..].reverse();
            }
        }
        Ok(diagnostics)
    }

    pub fn sequel_tree(
        &mut self,
        parser: &mut Parser,
        budget: &Budget,
    ) -> Result<Option<Tree>, String> {
        // Sequel splits doubled quotes, dollar signs and high identifier bytes.
        // Semantic names and byte ranges always come from the original SQL.
        for (range, byte) in self.identifiers.drain(..) {
            for chunk in self.bytes[range].chunks_mut(CHUNK) {
                budget.check()?;
                chunk.fill(byte);
            }
        }
        parse(parser, &self.bytes, budget)
    }
}

fn parse(parser: &mut Parser, bytes: &[u8], budget: &Budget) -> Result<Option<Tree>, String> {
    let mut progress = |_: &tree_sitter::ParseState| {
        if budget.check().is_ok() {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        }
    };
    let options = ParseOptions::new().progress_callback(&mut progress);
    let tree = parser.parse_with_options(
        &mut |offset, _| -> &[u8] {
            if budget.check().is_err() {
                return &[];
            }
            let remaining = bytes.get(offset..).unwrap_or_default();
            &remaining[..remaining.len().min(CHUNK)]
        },
        None,
        Some(options),
    );
    budget.check()?;
    Ok(tree)
}

pub(super) fn identifier(s: &str) -> String {
    match s.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        Some(inner) => inner.replace("\"\"", "\""),
        None => s.to_ascii_lowercase(),
    }
}
