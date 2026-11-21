use std::{collections::HashMap, sync::Arc};

pub(super) struct Preview {
    pub headers: Arc<Vec<String>>,
    pub labels: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub edges: Vec<f64>,
    pub selection: Arc<Vec<Option<usize>>>,
    uses: Vec<usize>,
    pub ambiguous: std::collections::HashSet<usize>,
}

fn label(value: &str) -> String {
    let mut chars = value.chars();
    let mut text: String = chars.by_ref().take(80).collect();
    if chars.next().is_some() {
        text.push('…')
    }
    text
}

impl Preview {
    pub fn new<'a>(
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
        columns: impl Iterator<Item = &'a str>,
        current: impl Fn() -> bool,
    ) -> Result<Self, String> {
        let check = || {
            current()
                .then_some(())
                .ok_or_else(|| "CSV preview cancelled".to_owned())
        };
        let mut exact = HashMap::new();
        let mut folded = HashMap::new();
        let mut max_bytes = 0;
        for (i, name) in columns.enumerate() {
            check()?;
            exact.insert(name, i);
            let lower = name.to_lowercase();
            max_bytes = max_bytes.max(lower.len().saturating_mul(4));
            folded
                .entry(lower)
                .and_modify(|value| *value = None)
                .or_insert(Some(i));
        }
        let mut uses = vec![0; exact.len()];
        let mut ambiguous = std::collections::HashSet::new();
        let mut selection = Vec::with_capacity(headers.len());
        let mut labels = Vec::with_capacity(headers.len());
        for (i, header) in headers.iter().enumerate() {
            check()?;
            let target = if header.len() > max_bytes {
                None
            } else if let Some(&index) = exact.get(header.as_str()) {
                Some(index)
            } else {
                match folded.get(&header.to_lowercase()) {
                    Some(None) => {
                        ambiguous.insert(i);
                        None
                    }
                    matched => matched.copied().flatten(),
                }
            };
            if let Some(index) = target {
                uses[index] += 1;
            }
            selection.push(target);
            labels.push(label(header));
        }
        let rows: Vec<Vec<String>> = rows
            .into_iter()
            .map(|r| {
                r.into_iter()
                    .map(|v| {
                        check()?;
                        Ok(label(&v))
                    })
                    .collect::<Result<Vec<_>, String>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut edges = Vec::with_capacity(labels.len() + 1);
        edges.push(0.);
        for (index, header) in labels.iter().enumerate() {
            check()?;
            let chars = std::iter::once(header)
                .chain(rows.iter().filter_map(|row| row.get(index)))
                .map(|value| value.encode_utf16().take(25).count())
                .max()
                .unwrap_or(0);
            let width = (24. + chars as f64 * 7.5).clamp(32., 200.);
            edges.push(edges[index] + width);
        }
        check()?;
        Ok(Self {
            headers: Arc::new(headers),
            labels,
            rows,
            edges,
            selection: Arc::new(selection),
            uses,
            ambiguous,
        })
    }

    pub fn set(&mut self, source: usize, target: Option<usize>) -> bool {
        if source >= self.selection.len() || target.is_some_and(|i| i >= self.uses.len()) {
            return false;
        }
        self.ambiguous.remove(&source);
        let old = std::mem::replace(&mut Arc::make_mut(&mut self.selection)[source], target);
        if let Some(i) = old {
            self.uses[i] -= 1;
        }
        if let Some(i) = target {
            self.uses[i] += 1;
        }
        true
    }

    pub fn problem(&self) -> Option<&'static str> {
        if !self.ambiguous.is_empty() {
            return Some("Choose a destination or Skip for each ambiguous header.");
        }
        if self.uses.iter().any(|&n| n > 1) {
            return Some("Choose each target column only once.");
        }
        if self.uses.iter().all(|&n| n == 0) {
            return Some("Map at least one column.");
        }
        None
    }
}
