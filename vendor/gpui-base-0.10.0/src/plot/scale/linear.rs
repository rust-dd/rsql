// @reference: https://d3js.org/d3-scale/linear

use itertools::Itertools;

use super::{PlotValue, Scale};

#[derive(Clone)]
pub struct ScaleLinear<T> {
    domain_start: T,
    domain_diff: T,
    range_start: f32,
    range_diff: f32,
}

impl<T: PlotValue> ScaleLinear<T> {
    /// Map the extent of `domain` onto `range`: the smallest domain value to
    /// `range[0]` and the largest to `range[1]`, so a reversed range such as
    /// `[height, 0.]` puts larger values higher.
    pub fn new(domain: impl IntoIterator<Item = T>, range: [f32; 2]) -> Self {
        let (domain_start, domain_end) = domain
            .into_iter()
            .minmax_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .into_option()
            .unwrap_or((T::zero(), T::zero()));

        Self {
            domain_start,
            domain_diff: domain_end - domain_start,
            range_start: range[0],
            range_diff: range[1] - range[0],
        }
    }
}

impl<T: PlotValue> Scale<T> for ScaleLinear<T> {
    fn tick(&self, value: &T) -> Option<f32> {
        if self.domain_diff.is_zero() {
            return None;
        }

        let ratio = ((*value - self.domain_start) / self.domain_diff).to_f32()?;

        Some(ratio * self.range_diff + self.range_start)
    }
}
