// @reference: https://d3js.org/d3-scale/point

use super::Scale;

/// Point scale maps discrete domain values to continuous range positions.
///
/// Points are evenly distributed across the range, with the first and last points
/// aligned to the range boundaries.
#[derive(Clone)]
pub struct ScalePoint<T> {
    domain: Vec<T>,
    range_start: f32,
    range_tick: f32,
}

impl<T> ScalePoint<T>
where
    T: PartialEq,
{
    /// Place `domain` evenly from `range[0]` to `range[1]`; a single value sits
    /// in the middle of the range.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let scale = ScalePoint::new([1, 2, 3], [0., 100.]);
    /// assert_eq!(scale.tick(&1), Some(0.));
    /// assert_eq!(scale.tick(&2), Some(50.));
    /// assert_eq!(scale.tick(&3), Some(100.));
    /// ```
    pub fn new(domain: impl IntoIterator<Item = T>, range: [f32; 2]) -> Self {
        let domain: Vec<T> = domain.into_iter().collect();
        let len = domain.len();
        let range_diff = range[1] - range[0];
        let (range_start, range_tick) = match len {
            0 => (0., 0.),
            1 => (range[0], range_diff),
            _ => (range[0], range_diff / (len - 1) as f32),
        };

        Self {
            domain,
            range_start,
            range_tick,
        }
    }

    /// Returns the position of the domain value at `index`.
    ///
    /// Equivalent to [`Scale::tick`] on `domain[index]` for a domain of unique
    /// values, without searching the domain. Charts whose domain is built one
    /// entry per datum use this to project a series in linear time.
    pub fn tick_at(&self, index: usize) -> Option<f32> {
        let len = self.domain.len();
        if index >= len {
            return None;
        }

        if len == 1 {
            Some(self.range_start + self.range_tick / 2.)
        } else {
            Some(self.range_start + index as f32 * self.range_tick)
        }
    }
}

impl<T> Scale<T> for ScalePoint<T>
where
    T: PartialEq,
{
    fn tick(&self, value: &T) -> Option<f32> {
        let index = self.domain.iter().position(|v| v == value)?;
        self.tick_at(index)
    }

    fn nearest_index(&self, tick: f32) -> usize {
        if self.domain.is_empty() {
            return 0;
        }

        if self.range_tick == 0. {
            return 0;
        }

        let normalized_tick = tick - self.range_start;
        let index = (normalized_tick / self.range_tick).round() as usize;
        index.min(self.domain.len() - 1)
    }
}
