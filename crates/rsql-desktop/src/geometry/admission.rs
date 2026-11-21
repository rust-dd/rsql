//! Admission before geozero's eager text readers start allocating a geometry tree.
use super::budget::{Pool, Reservation};
use super::{GeometryBudget, GeometryError, MAX_COORDINATES, Resource, check_cancel};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::borrow::Cow;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Shape {
    Point,
    Line,
    Polygon,
}

impl Shape {
    fn json_name(self) -> &'static str {
        match self {
            Self::Point => "Point",
            Self::Line => "LineString",
            Self::Polygon => "Polygon",
        }
    }
}

const PAGE: usize = 4096;
const FIXED: usize = 4096;

/// WKT 0.14 holds 48-byte XY/Z/M coordinates in growing Vecs and tokenizes one word at a time.
pub(super) fn wkt(
    text: &str,
    budget: &GeometryBudget,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Reservation, GeometryError> {
    let mut depth = 0usize;
    let mut groups = 0usize;
    let mut coordinates = 1usize;
    for (chunk_index, chunk) in text.as_bytes().chunks(PAGE).enumerate() {
        check_cancel(cancelled)?;
        for (index, byte) in chunk.iter().enumerate() {
            let next = chunk_index * PAGE + index + 1;
            match byte {
                b'(' => {
                    groups += 1;
                    depth += 1;
                    if depth > 2 {
                        return Err(GeometryError::Malformed);
                    }
                }
                b')' => {
                    depth = depth.checked_sub(1).ok_or(GeometryError::Malformed)?;
                    // The WKT reader stops at the first complete geometry.
                    if depth == 0 && next != text.len() {
                        return Err(GeometryError::Malformed);
                    }
                }
                b',' => {
                    coordinates += 1;
                    if coordinates > MAX_COORDINATES {
                        return Err(GeometryError::Resource(Resource::Coordinates));
                    }
                }
                // wkt strips a leading plus before parsing the signed number.
                b'+' if matches!(text.as_bytes().get(next), Some(b'+' | b'-')) => {
                    return Err(GeometryError::Malformed);
                }
                0 => return Err(GeometryError::Malformed),
                _ => {}
            }
        }
    }
    if depth != 0 || groups == 0 {
        return Err(GeometryError::Malformed);
    }
    // Old and new capacities during doubling total at most 3n. Each group also gets
    // initial capacity (four coordinates) and its ring handle in the growing polygon.
    let bytes = FIXED + text.len() * 4 + coordinates * 144 + groups.saturating_mul(384);
    budget.reserve(Pool::Parser, bytes)
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(rename = "type", borrow)]
    kind: Cow<'a, str>,
    #[serde(borrow)]
    coordinates: Option<&'a RawValue>,
}

pub(super) struct PreparedJson {
    pub(super) shape: Shape,
    pub(super) text: String,
    _reservation: Reservation,
}

pub(super) fn json(
    text: &str,
    budget: &GeometryBudget,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<PreparedJson, GeometryError> {
    // RawValue and ignored members use serde_json's iterative skip stack. Escaped
    // keys/type and the reusable scratch may each grow with the source byte length.
    let _envelope = budget.reserve(Pool::Parser, FIXED + text.len() * 8)?;
    check_cancel(cancelled)?;
    let envelope = serde_json::from_str::<Envelope>(text);
    check_cancel(cancelled)?;
    let value = envelope.map_err(|_| GeometryError::Malformed)?;
    let shape = match value.kind.as_ref() {
        "Point" => Shape::Point,
        "LineString" => Shape::Line,
        "Polygon" => Shape::Polygon,
        _ => return Err(GeometryError::Unsupported),
    };
    let coords = value.coordinates.ok_or(GeometryError::Malformed)?.get();
    let bytes = json_tree_bytes(coords, shape, cancelled)?;
    let reservation = budget.reserve(Pool::Parser, bytes)?;
    // Only the coordinate payload enters the eager reader. This excludes foreign
    // member trees and prevents geozero from flattening feature wrappers.
    let normalized = format!(
        r#"{{"type":"{}","coordinates":{coords}}}"#,
        shape.json_name()
    );
    check_cancel(cancelled)?;
    Ok(PreparedJson {
        shape,
        text: normalized,
        _reservation: reservation,
    })
}

fn json_tree_bytes(
    coords: &str,
    shape: Shape,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<usize, GeometryError> {
    let coordinate_depth = match shape {
        Shape::Point => 1,
        Shape::Line => 2,
        Shape::Polygon => 3,
    };
    let mut depth = 0usize;
    let mut arrays = 0usize;
    let mut commas = 0usize;
    let mut positions = 0usize;
    for chunk in coords.as_bytes().chunks(PAGE) {
        check_cancel(cancelled)?;
        for &byte in chunk {
            match byte {
                b'[' => {
                    arrays += 1;
                    depth += 1;
                    if depth > coordinate_depth {
                        return Err(GeometryError::Malformed);
                    }
                    if depth == coordinate_depth {
                        positions += 1;
                        if positions > MAX_COORDINATES {
                            return Err(GeometryError::Resource(Resource::Coordinates));
                        }
                    }
                }
                b']' => depth = depth.checked_sub(1).ok_or(GeometryError::Malformed)?,
                b',' => commas += 1,
                b'0'..=b'9' | b'.' | b'-' | b'+' | b'E' | b'e' | b' ' | b'\t' | b'\r' | b'\n' => {}
                _ => return Err(GeometryError::Malformed),
            }
        }
    }
    // Each nonempty array contributes at most commas + 1 elements. Account for
    // its minimum four Value slots, old + new backing stores, numeric strings,
    // typed geojson Vecs, the normalized reader copies and number-token scratch.
    let slots = (arrays + commas + 1).saturating_mul(3) + arrays * 4;
    let slot_bytes = size_of::<serde_json::Value>() + size_of::<Vec<f64>>() + 16;
    Ok((FIXED + coords.len() * 12).saturating_add(slots.saturating_mul(slot_bytes)))
}
