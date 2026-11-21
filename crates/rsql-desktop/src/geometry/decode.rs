use super::admission::{self, Shape};
use super::budget::Pool;
use super::detect::wkt_prefix;
use super::{
    Geometry, GeometryBudget, GeometryData, GeometryError, MAX_COORDINATES, MAX_INPUT_BYTES,
    Resource, check_cancel,
};
use geozero::error::{GeozeroError, Result as ProcessResult};
use geozero::{GeomProcessor, GeozeroGeometry};
use rsql_core::wire::Cell;

/// Borrows one source cell for the duration of this synchronous worker operation.
/// Polls cancellation during admission and between library events. An eager text
/// reader already executing must return before its parser reservation is released.
pub fn decode(
    value: &Cell,
    row_index: usize,
    budget: &GeometryBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Option<Geometry>, GeometryError> {
    check_cancel(&mut cancelled)?;
    let Some(text) = value.as_deref() else {
        return Ok(None);
    };
    if text.len() > MAX_INPUT_BYTES {
        return Err(GeometryError::Resource(Resource::InputBytes));
    }
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.starts_with('{') {
        let json = admission::json(text, budget, &mut cancelled)?;
        return process(
            geozero::geojson::GeoJson(&json.text),
            json.shape,
            row_index,
            budget,
            &mut cancelled,
        )
        .map(Some);
    }
    if let Some(prefix) = wkt_prefix(text) {
        let shape = match prefix {
            "POINT" => Shape::Point,
            "LINESTRING" => Shape::Line,
            "POLYGON" => Shape::Polygon,
            _ => return Err(GeometryError::Unsupported),
        };
        let _tree = admission::wkt(text, budget, &mut cancelled)?;
        return process(
            geozero::wkt::Wkt(text),
            shape,
            row_index,
            budget,
            &mut cancelled,
        )
        .map(Some);
    }
    if text.as_bytes()[0].is_ascii_digit()
        || (text.len() >= 8 && text.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return ewkb(text, row_index, budget, &mut cancelled).map(Some);
    }
    Err(GeometryError::Unsupported)
}

fn ewkb(
    text: &str,
    row_index: usize,
    budget: &GeometryBudget,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<Geometry, GeometryError> {
    if !text.len().is_multiple_of(2) {
        return Err(GeometryError::Malformed);
    }
    let mut header = [0u8; 5];
    hex::decode_to_slice(
        text.as_bytes().get(..10).ok_or(GeometryError::Malformed)?,
        &mut header,
    )
    .map_err(|_| GeometryError::Malformed)?;
    let encoded_len = match header {
        [1, 1, 0, 0, 0] => 42,
        [1, 1, 0, 0, 32] => 50,
        [0 | 1, ..] => return Err(GeometryError::Unsupported),
        _ => return Err(GeometryError::Malformed),
    };
    if text.len() != encoded_len {
        return Err(GeometryError::Malformed);
    }
    let mut bytes = [0; 25];
    let raw = &mut bytes[..encoded_len / 2];
    hex::decode_to_slice(text, &mut *raw).map_err(|_| GeometryError::Malformed)?;
    process(
        geozero::wkb::Ewkb(raw),
        Shape::Point,
        row_index,
        budget,
        cancelled,
    )
}

fn process(
    input: impl GeozeroGeometry,
    shape: Shape,
    row_index: usize,
    budget: &GeometryBudget,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<Geometry, GeometryError> {
    check_cancel(cancelled)?;
    let mut sink = Sink {
        row_index,
        budget,
        cancelled,
        data: None,
        failure: None,
        outer: true,
        total: 0,
    };
    let result = input.process_geom(&mut sink);
    if let Some(failure) = sink.failure {
        return Err(failure);
    }
    check_cancel(sink.cancelled)?;
    result.map_err(|_| GeometryError::Malformed)?;
    let data = sink.data.ok_or(GeometryError::Malformed)?;
    let coords = &data.coordinates;
    match shape {
        Shape::Point if coords.len() == 1 => Ok(Geometry::Point(data)),
        Shape::Line if coords.len() >= 2 => Ok(Geometry::LineString(data)),
        Shape::Polygon if coords.len() >= 4 && coords.first() == coords.last() => {
            Ok(Geometry::Polygon(data))
        }
        _ => Err(GeometryError::Malformed),
    }
}

struct Sink<'a> {
    row_index: usize,
    budget: &'a GeometryBudget,
    cancelled: &'a mut dyn FnMut() -> bool,
    data: Option<GeometryData>,
    failure: Option<GeometryError>,
    outer: bool,
    total: usize,
}

impl Sink<'_> {
    fn event(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<(), GeometryError>,
    ) -> ProcessResult<()> {
        check_cancel(self.cancelled)
            .and_then(|()| f(self))
            .map_err(|err| {
                self.failure = Some(err);
                GeozeroError::GeometryFormat
            })
    }

    fn begin(&mut self, count: usize) -> Result<(), GeometryError> {
        if count > MAX_COORDINATES - self.total {
            return Err(GeometryError::Resource(Resource::Coordinates));
        }
        self.total += count;
        if self.outer {
            // Charge the enum and one Arc owner (two counters and one handle),
            // including Vec and reservation metadata, before allocating coordinates.
            let overhead = size_of::<Geometry>() + 3 * size_of::<usize>();
            let bytes = count * size_of::<[f64; 2]>() + overhead;
            let reservation = self.budget.reserve(Pool::Retained, bytes)?;
            let mut coordinates = Vec::new();
            coordinates
                .try_reserve_exact(count)
                .map_err(|_| GeometryError::Resource(Resource::RetainedBytes))?;
            if coordinates.capacity() != count {
                return Err(GeometryError::Resource(Resource::RetainedBytes));
            }
            self.data = Some(GeometryData {
                row_index: self.row_index,
                coordinates,
                _reservation: reservation,
            });
        }
        Ok(())
    }
}

impl GeomProcessor for Sink<'_> {
    fn point_begin(&mut self, _idx: usize) -> ProcessResult<()> {
        self.event(|s| s.begin(1))
    }

    fn linestring_begin(&mut self, tagged: bool, size: usize, idx: usize) -> ProcessResult<()> {
        self.event(|s| {
            s.outer = tagged || idx == 0;
            s.begin(size)
        })
    }

    fn xy(&mut self, x: f64, y: f64, _idx: usize) -> ProcessResult<()> {
        self.event(|s| {
            if s.outer {
                let coords = &mut s.data.as_mut().ok_or(GeometryError::Malformed)?.coordinates;
                if !x.is_finite() || !y.is_finite() || coords.len() == coords.capacity() {
                    return Err(GeometryError::Malformed);
                }
                coords.push([x, y]);
            }
            Ok(())
        })
    }
}
