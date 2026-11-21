mod admission;
mod budget;
mod decode;
mod detect;
mod projection;

pub use budget::GeometryBudget;
pub use decode::decode;
pub use detect::{detect_column, detect_sample};
pub use projection::{Bounds, MapTransform, continuous_world, longitude_latitude, world};

use budget::Reservation;

pub const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_COORDINATES: usize = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    InputBytes,
    Coordinates,
    RetainedBytes,
    ParserBytes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryError {
    Unsupported,
    Malformed,
    Resource(Resource),
    Cancelled,
}

/// Share the whole geometry with an Arc to keep its charge alive across views.
#[derive(Debug)]
pub enum Geometry {
    Point(GeometryData),
    LineString(GeometryData),
    Polygon(GeometryData),
}

#[derive(Debug)]
pub struct GeometryData {
    row_index: usize,
    coordinates: Vec<[f64; 2]>,
    _reservation: Reservation,
}

impl Geometry {
    fn data(&self) -> &GeometryData {
        match self {
            Self::Point(data) | Self::LineString(data) | Self::Polygon(data) => data,
        }
    }

    pub fn row_index(&self) -> usize {
        self.data().row_index
    }

    /// Source longitude and latitude, before wrapping or projection.
    pub fn coordinates(&self) -> &[[f64; 2]] {
        &self.data().coordinates
    }
}

fn check_cancel(cancelled: &mut (impl FnMut() -> bool + ?Sized)) -> Result<(), GeometryError> {
    if cancelled() {
        return Err(GeometryError::Cancelled);
    }
    Ok(())
}
