use std::ops::Range;

#[derive(Default)]
pub(super) struct Utf16Index {
    bytes: Vec<usize>,
    end: usize,
}

pub(super) struct TextRange {
    pub bytes: Range<usize>,
    pub units: Range<usize>,
}

impl Utf16Index {
    pub fn new(text: &str) -> Self {
        let mut bytes = Vec::with_capacity(text.len());
        for (byte, c) in text.char_indices() {
            bytes.extend(std::iter::repeat_n(byte, c.len_utf16()));
        }
        Self {
            bytes,
            end: text.len(),
        }
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn byte(&self, unit: usize) -> usize {
        self.bytes.get(unit).copied().unwrap_or(self.end)
    }

    pub fn unit(&self, byte: usize) -> usize {
        self.bytes.partition_point(|start| *start < byte)
    }

    fn inside_surrogate(&self, unit: usize) -> bool {
        unit > 0 && unit < self.len() && self.bytes[unit] == self.bytes[unit - 1]
    }

    pub fn range(&self, range: Range<usize>) -> TextRange {
        let mut start = range.start.min(self.len());
        let mut end = range.end.max(range.start).min(self.len());
        if self.inside_surrogate(start) {
            start -= 1;
        }
        if self.inside_surrogate(end) {
            if range.end > range.start {
                end += 1;
            } else {
                end -= 1;
            }
        }
        TextRange {
            bytes: self.byte(start)..self.byte(end),
            units: start..end,
        }
    }
}
