// @reference: https://d3js.org/d3-scale/ordinal

#[derive(Clone)]
pub struct ScaleOrdinal<D, R> {
    domain: Vec<D>,
    range: Vec<R>,
    unknown: Option<R>,
}

impl<D, R> Default for ScaleOrdinal<D, R> {
    fn default() -> Self {
        Self {
            domain: Vec::new(),
            range: Vec::new(),
            unknown: None,
        }
    }
}

impl<D, R> ScaleOrdinal<D, R> {
    pub fn new(domain: Vec<D>, range: Vec<R>) -> Self {
        Self {
            domain,
            range,
            unknown: None,
        }
    }

    /// Set the domain to the specified array of values.
    pub fn domain(mut self, domain: Vec<D>) -> Self {
        self.domain = domain;
        self
    }

    /// Set the range of the ordinal scale to the specified array of values.
    pub fn range(mut self, range: Vec<R>) -> Self {
        self.range = range;
        self
    }

    /// Set the output value of the scale for unknown input values and returns this scale.
    pub fn unknown(mut self, unknown: R) -> Self {
        self.unknown = Some(unknown);
        self
    }
}

impl<D, R> ScaleOrdinal<D, R>
where
    D: PartialEq,
    R: Clone,
{
    /// Given a value in the input domain, returns the corresponding value in the output range.
    pub fn map(&self, value: &D) -> Option<R> {
        if let Some(index) = self.domain.iter().position(|v| v == value) {
            if self.range.is_empty() {
                None
            } else {
                Some(self.range[index % self.range.len()].clone())
            }
        } else {
            self.unknown.clone()
        }
    }
}
