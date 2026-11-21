// @reference: https://d3js.org/d3-shape/pie

use std::f32::consts::TAU;

use super::arc::ArcData;

#[allow(clippy::type_complexity)]
pub struct Pie<T> {
    value: Box<dyn Fn(&T) -> Option<f32>>,
    start_angle: f32,
    end_angle: f32,
    pad_angle: f32,
}

impl<T> Default for Pie<T> {
    fn default() -> Self {
        Self {
            value: Box::new(|_| None),
            start_angle: 0.,
            end_angle: TAU,
            pad_angle: 0.,
        }
    }
}

impl<T> Pie<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the value of the Pie.
    pub fn value<F>(mut self, value: F) -> Self
    where
        F: 'static + Fn(&T) -> Option<f32>,
    {
        self.value = Box::new(value);
        self
    }

    /// Set the start angle of the Pie.
    pub fn start_angle(mut self, start_angle: f32) -> Self {
        self.start_angle = start_angle;
        self
    }

    /// Set the end angle of the Pie.
    pub fn end_angle(mut self, end_angle: f32) -> Self {
        self.end_angle = end_angle;
        self
    }

    /// Set the pad angle of the Pie.
    pub fn pad_angle(mut self, pad_angle: f32) -> Self {
        self.pad_angle = pad_angle;
        self
    }

    /// Get the arcs of the Pie.
    pub fn arcs<'a>(&self, data: &'a [T]) -> Vec<ArcData<'a, T>> {
        let mut values = Vec::new();
        let mut sum = 0.;

        for (idx, v) in data.iter().enumerate() {
            if let Some(value) = (self.value)(v) {
                if value > 0. {
                    sum += value;
                    values.push((idx, v, value));
                }
            }
        }

        let mut arcs = Vec::with_capacity(values.len());
        let mut k = self.start_angle;

        for (index, v, value) in values {
            let start_angle = k;
            let angle_delta = if sum > 0. {
                (value / sum) * (self.end_angle - self.start_angle)
            } else {
                0.
            };
            k += angle_delta;
            let end_angle = k;

            arcs.push(ArcData {
                data: v,
                index,
                value,
                start_angle,
                end_angle,
                pad_angle: self.pad_angle,
            });
        }

        arcs
    }
}
