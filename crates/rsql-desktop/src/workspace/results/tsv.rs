use super::*;
use csv_core::{ReadFieldResult, ReaderBuilder};

const CHUNK: usize = 8192;

struct Entries<I> {
    entries: I,
    text: String,
    offset: usize,
    saw_text: bool,
}

impl<I: Iterator<Item = ClipboardEntry>> Entries<I> {
    fn chunk(&mut self, current: &impl Fn() -> bool) -> Result<&str, String> {
        while self.offset == self.text.len() {
            if !current() {
                return Err("Cancelled".into());
            }
            let Some(entry) = self.entries.next() else {
                return Ok("");
            };
            if let ClipboardEntry::String(s) = entry {
                self.saw_text = true;
                self.text = s.into_text();
                self.offset = 0;
            }
        }
        let tail = &self.text[self.offset..];
        Ok(&tail[..tail.floor_char_boundary(CHUNK)])
    }
}

// Chunked storage avoids moving a growing multi-megabyte field between owner checks.
#[derive(Default)]
pub(super) struct Field {
    chunks: Vec<String>,
    len: usize,
}

impl Field {
    pub(super) fn push(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.len += text.len();
        if let Some(last) = self.chunks.last_mut()
            && last.len() + text.len() <= CHUNK
        {
            last.push_str(text);
        } else {
            self.chunks.push(text.to_owned());
        }
    }

    pub(super) fn finish(mut self, current: &impl Fn() -> bool) -> Result<String, String> {
        if self.chunks.len() == 1 {
            return Ok(self.chunks.pop().unwrap());
        }
        let mut result = String::with_capacity(self.len);
        for part in self.chunks {
            if !current() {
                return Err("Cancelled".into());
            }
            result.push_str(&part);
        }
        Ok(result)
    }
}

pub(super) fn paste(
    item: ClipboardItem,
    data: &GridData,
    start: Position,
    current: &impl Fn() -> bool,
) -> Result<Vec<(Position, Cell)>, String> {
    let mut source = Entries {
        entries: item.into_entries(),
        text: String::new(),
        offset: 0,
        saw_text: false,
    };
    source.chunk(current)?;
    let mut edits = Vec::new();
    if !source.saw_text {
        return Ok(edits);
    }
    let cols = data.result.columns().len();
    let mut reader = ReaderBuilder::new().delimiter(b'\t').build();
    let mut output = [0u8; CHUNK];
    for row in start.row..data.rows() {
        // The CSV reader normally skips blank records and a leading BOM. A synthetic
        // first field makes both ordinary clipboard data, including one empty string.
        reader.reset();
        reader.read_field(b"\t", &mut output);
        let mut col = start.col;
        loop {
            if col >= cols && row + 1 == data.rows() {
                return Ok(edits);
            }
            let target = data.retained(Position { row, col }).filter(|p| {
                data.access.allows(*p)
                    && matches!(data.result.cache.cell(p.row, p.col), CellState::Value(_))
            });
            let mut field = Field::default();
            let ended_cr = loop {
                if !current() {
                    return Err("Cancelled".into());
                }
                let chunk = source.chunk(current)?;
                let (status, used, written) = reader.read_field(chunk.as_bytes(), &mut output);
                let cr = chunk[..used].ends_with('\r');
                source.offset += used;
                if target.is_some() {
                    // Input chunks end at UTF-8 boundaries; CSV removes only ASCII syntax.
                    field.push(std::str::from_utf8(&output[..written]).map_err(|e| e.to_string())?);
                }
                match status {
                    ReadFieldResult::InputEmpty | ReadFieldResult::OutputFull => continue,
                    ReadFieldResult::End => return Ok(edits),
                    ReadFieldResult::Field { record_end } => {
                        if let Some(pos) = target {
                            edits.push((pos, Some(field.finish(current)?)));
                        }
                        break record_end.then_some(cr);
                    }
                }
            };
            if let Some(cr) = ended_cr {
                if cr && source.chunk(current)?.starts_with('\n') {
                    source.offset += 1;
                }
                break;
            }
            col += 1;
        }
        if source.chunk(current)?.is_empty() {
            break;
        }
    }
    Ok(edits)
}
