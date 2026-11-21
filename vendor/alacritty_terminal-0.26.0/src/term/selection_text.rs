use super::{Column, Dimensions, Line, Point, SelectionRange, SelectionType, Term};

/// Selected text exceeds the caller's byte limit.
#[derive(Debug, PartialEq, Eq)]
pub struct SelectionTooLarge;

impl<T> Term<T> {
    /// Convert the active selection to a String.
    pub fn selection_to_string(&self) -> Option<String> {
        self.selection_to_string_limited(usize::MAX).ok().flatten()
    }

    /// Copy using native selection semantics, returning an error if the result exceeds the byte limit.
    pub fn selection_to_string_limited(
        &self,
        limit: usize,
    ) -> Result<Option<String>, SelectionTooLarge> {
        let Some(selection) = &self.selection else {
            return Ok(None);
        };
        let Some(SelectionRange { start, end, .. }) = selection.to_range(self) else {
            return Ok(None);
        };
        let mut res = String::new();
        if selection.ty == SelectionType::Block {
            for line in (start.line.0..=end.line.0).map(Line::from) {
                let text = self.line_to_string(
                    line,
                    start.column..end.column,
                    line == end.line || start.column.0 != 0,
                    &mut false,
                );
                let text = text.trim_end();
                let newline = usize::from(line != end.line);
                if text.len().saturating_add(newline) > limit - res.len() {
                    return Err(SelectionTooLarge);
                }
                res.push_str(text);
                if newline != 0 {
                    res.push('\n');
                }
            }
        } else {
            let lines = selection.ty == SelectionType::Lines;
            let limit = limit
                .checked_sub(usize::from(lines))
                .ok_or(SelectionTooLarge)?;
            res = self.bounds_to_string_limited(start, end, limit)?;
            if lines {
                res.push('\n');
            }
        }
        Ok(Some(res))
    }

    /// Convert range between two points to a String.
    pub fn bounds_to_string(&self, start: Point, end: Point) -> String {
        self.bounds_to_string_limited(start, end, usize::MAX)
            .unwrap()
    }

    fn bounds_to_string_limited(
        &self,
        start: Point,
        end: Point,
        limit: usize,
    ) -> Result<String, SelectionTooLarge> {
        let mut res = String::new();
        let mut in_tab = false;
        for line in (start.line.0..=end.line.0).map(Line::from) {
            let start_col = if line == start.line {
                start.column
            } else {
                Column(0)
            };
            let end_col = if line == end.line {
                end.column
            } else {
                self.last_column()
            };
            let text = self.line_to_string(line, start_col..end_col, line == end.line, &mut in_tab);
            let possible = limit.saturating_add(1);
            if text.len() > possible - res.len() {
                return Err(SelectionTooLarge);
            }
            res.push_str(&text);
        }
        if res.ends_with('\n') {
            res.pop();
        }
        if res.len() > limit {
            return Err(SelectionTooLarge);
        }
        Ok(res)
    }
}
