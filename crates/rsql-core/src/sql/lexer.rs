use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenKind {
    Word,
    Integer,
    QuotedIdentifier,
    String,
    Comment,
    Symbol,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub range: Range<usize>,
    pub complete: bool,
}

/// ECMAScript whitespace used by the legacy editor's split and trim operations.
pub fn editor_whitespace(c: char) -> bool {
    (c.is_whitespace() && c != '\u{0085}') || c == '\u{feff}'
}

pub fn sql_whitespace(c: char) -> bool {
    c.is_ascii_whitespace() || c == '\u{b}'
}

fn ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic() || !c.is_ascii()
}

const CHECK_INTERVAL: usize = 8192;

struct Scanner<'a, F> {
    bytes: &'a [u8],
    at: usize,
    next_check: usize,
    after_escape: bool,
    newline: bool,
    check: F,
}

impl<F, E> Scanner<'_, F>
where
    F: FnMut() -> Result<(), E>,
{
    fn advance(&mut self, n: usize) -> Result<(), E> {
        self.at = self.bytes.len().min(self.at + n);
        if self.at >= self.next_check {
            (self.check)()?;
            self.next_check = self.at + CHECK_INTERVAL;
        }
        Ok(())
    }

    fn take_while(&mut self, accept: impl Fn(u8) -> bool) -> Result<(), E> {
        while self.bytes.get(self.at).copied().is_some_and(&accept) {
            self.advance(1)?;
        }
        Ok(())
    }

    fn quoted(&mut self, quote: u8, escaped: bool) -> Result<bool, E> {
        self.advance(1)?;
        while let Some(&c) = self.bytes.get(self.at) {
            self.advance(1)?;
            if escaped && c == b'\\' {
                self.advance(1)?;
            } else if c == quote {
                if self.bytes.get(self.at) != Some(&quote) {
                    return Ok(true);
                }
                self.advance(1)?;
            }
        }
        Ok(false)
    }

    fn block_comment(&mut self) -> Result<bool, E> {
        let mut depth = 1;
        self.advance(2)?;
        while self.at < self.bytes.len() {
            match self.bytes.get(self.at..self.at + 2) {
                Some(b"/*") => {
                    self.advance(2)?;
                    depth += 1;
                }
                Some(b"*/") => {
                    self.advance(2)?;
                    depth -= 1;
                    if depth == 0 {
                        return Ok(true);
                    }
                }
                _ => self.advance(1)?,
            }
        }
        Ok(false)
    }

    fn dollar(&mut self, start: usize) -> Result<(TokenKind, bool), E> {
        self.advance(1)?;
        if self.bytes.get(self.at).copied().is_some_and(ident_start) {
            self.take_while(|c| ident_start(c) || c.is_ascii_digit())?;
        }
        if self.bytes.get(self.at) != Some(&b'$') {
            self.at = start + 1;
            self.next_check = self.at;
            return Ok((TokenKind::Symbol, true));
        }
        self.advance(1)?;
        let tag = &self.bytes[start..self.at];
        while self.at < self.bytes.len() {
            if self.bytes[self.at] == b'$'
                && let Some(candidate) = self.bytes.get(self.at..self.at + tag.len())
            {
                let mut matches = true;
                for (left, right) in candidate
                    .chunks(CHECK_INTERVAL)
                    .zip(tag.chunks(CHECK_INTERVAL))
                {
                    if left != right {
                        matches = false;
                        break;
                    }
                    if tag.len() > CHECK_INTERVAL {
                        (self.check)()?;
                    }
                }
                if matches {
                    self.advance(tag.len())?;
                    return Ok((TokenKind::String, true));
                }
            }
            self.advance(1)?;
        }
        Ok((TokenKind::String, false))
    }

    fn next(&mut self) -> Result<Option<Token>, E> {
        while let Some(&c) = self
            .bytes
            .get(self.at)
            .filter(|c| sql_whitespace(char::from(**c)))
        {
            if c == 0x0b {
                self.after_escape = false;
            }
            self.newline |= matches!(c, b'\r' | b'\n');
            self.advance(1)?;
        }
        let start = self.at;
        let Some(&c) = self.bytes.get(start) else {
            return Ok(None);
        };
        let line_comment = self.bytes.get(start..start + 2) == Some(b"--");
        let explicit = matches!(c, b'e' | b'E') && self.bytes.get(start + 1) == Some(&b'\'');
        let escaped = c == b'\'' && self.after_escape && self.newline;
        if !line_comment {
            self.after_escape = explicit || escaped;
            self.newline = false;
        }
        let (kind, complete) = match c {
            b'-' if self.bytes.get(start + 1) == Some(&b'-') => {
                self.take_while(|v| !matches!(v, b'\r' | b'\n'))?;
                (TokenKind::Comment, true)
            }
            b'/' if self.bytes.get(start + 1) == Some(&b'*') => {
                (TokenKind::Comment, self.block_comment()?)
            }
            b'\'' => (TokenKind::String, self.quoted(c, escaped)?),
            b'"' => (TokenKind::QuotedIdentifier, self.quoted(c, false)?),
            b'E' | b'e' if self.bytes.get(start + 1) == Some(&b'\'') => {
                self.advance(1)?;
                (TokenKind::String, self.quoted(b'\'', true)?)
            }
            b'$' => self.dollar(start)?,
            b'0'..=b'9' => {
                self.take_while(|v| v.is_ascii_digit())?;
                (TokenKind::Integer, true)
            }
            _ if ident_start(c) => {
                self.take_while(|v| ident_start(v) || v.is_ascii_digit() || v == b'$')?;
                (TokenKind::Word, true)
            }
            _ => {
                self.advance(1)?;
                (TokenKind::Symbol, true)
            }
        };
        Ok(Some(Token {
            kind,
            complete,
            range: start..self.at,
        }))
    }
}

pub fn scan<E>(
    sql: &str,
    check: impl FnMut() -> Result<(), E>,
    mut emit: impl FnMut(Token) -> Result<(), E>,
) -> Result<(), E> {
    let mut scanner = Scanner {
        bytes: sql.as_bytes(),
        at: 0,
        next_check: 0,
        after_escape: false,
        newline: false,
        check,
    };
    while let Some(token) = scanner.next()? {
        emit(token)?;
    }
    Ok(())
}

/// Byte ranges omit inter-token whitespace. An open quoted token or block comment extends to EOF.
pub fn lex(sql: &str) -> Vec<Token> {
    lex_with(sql, || Ok::<_, std::convert::Infallible>(())).unwrap()
}

pub fn lex_with<E>(sql: &str, check: impl FnMut() -> Result<(), E>) -> Result<Vec<Token>, E> {
    let mut tokens = Vec::new();
    scan(sql, check, |t| {
        tokens.push(t);
        Ok(())
    })?;
    Ok(tokens)
}
