// RSQL local source changes. See NOTICE for upstream provenance and modifications.
use super::InputLine;
use gpui::TextRun;
use smallvec::SmallVec;
use std::{
    cell::OnceCell,
    ops::{Index, Range},
};

pub(in crate::input) struct IndexedRuns {
    runs: Vec<TextRun>,
    ends: Vec<usize>,
}
impl IndexedRuns {
    pub(in crate::input) fn new(runs: Vec<TextRun>) -> Self {
        #[cfg(feature = "test-support")]
        crate::input::observations::measure(|work| work.indexed_styles += runs.len());
        let mut end = 0;
        let ends = runs
            .iter()
            .map(|run| {
                end += run.len;
                end
            })
            .collect();
        Self { runs, ends }
    }
    pub(in crate::input) fn slice(&self, range: Range<usize>) -> Vec<TextRun> {
        let first = self.ends.partition_point(|end| {
            #[cfg(feature = "test-support")]
            crate::input::observations::measure(|work| work.comparisons += 1);
            *end <= range.start
        });
        let mut start = if first == 0 { 0 } else { self.ends[first - 1] };
        self.runs[first..]
            .iter()
            .zip(&self.ends[first..])
            .map_while(|(run, &end)| {
                if start >= range.end {
                    return None;
                }
                #[cfg(feature = "test-support")]
                crate::input::observations::measure(|work| work.intersecting_styles += 1);
                let len = range.end.min(end) - range.start.max(start);
                start = end;
                Some(TextRun { len, ..run.clone() })
            })
            .collect()
    }
}
pub(crate) enum Rows {
    All(SmallVec<[InputLine; 1]>),
    Viewport {
        ranges: Vec<Range<usize>>,
        visible: Range<usize>,
        shaped: Vec<OnceCell<Box<InputLine>>>,
        build: Box<dyn Fn(usize) -> InputLine>,
    },
}
impl Rows {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::All(v) => v.len(),
            Self::Viewport { ranges, .. } => ranges.len(),
        }
    }
    pub(crate) fn visible(&self) -> Range<usize> {
        match self {
            Self::All(v) => 0..v.len(),
            Self::Viewport { visible, .. } => visible.clone(),
        }
    }
    pub(crate) fn iter(&self) -> impl Iterator<Item = &InputLine> {
        (0..self.len()).map(|i| &self[i])
    }
    pub(crate) fn last(&self) -> Option<&InputLine> {
        self.len().checked_sub(1).map(|i| &self[i])
    }
    pub(crate) fn start(&self, i: usize) -> usize {
        match self {
            Self::Viewport { ranges, .. } => ranges[i].start,
            Self::All(rows) => rows[..i].iter().map(|r| r.len).sum(),
        }
    }
    pub(crate) fn viewport(
        ranges: Vec<Range<usize>>,
        visible: Range<usize>,
        build: impl Fn(usize) -> InputLine + 'static,
    ) -> Self {
        let shaped = (0..ranges.len()).map(|_| OnceCell::new()).collect();
        Self::Viewport {
            ranges,
            visible,
            shaped,
            build: Box::new(build),
        }
    }
}
impl Index<usize> for Rows {
    type Output = InputLine;
    fn index(&self, index: usize) -> &InputLine {
        match self {
            Self::All(v) => &v[index],
            Self::Viewport { shaped, build, .. } => {
                shaped[index].get_or_init(|| Box::new(build(index)))
            }
        }
    }
}
