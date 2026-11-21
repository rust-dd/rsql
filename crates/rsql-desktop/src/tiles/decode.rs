use super::{Result, TILE_BYTES};
use gpui_kit::RenderImage;
use png::{ColorType, DecodeOptions, Decoder, Limits, Transformations};
use std::{io::Cursor, sync::Arc};
use tokio::sync::OwnedSemaphorePermit;

/// All consumer and future paint/atlas ownership must retain this handle.
/// The render image stays private so a naked Arc cannot escape its reservation.
#[derive(Debug)]
pub struct TileImage {
    image: Arc<RenderImage>,
    _reservation: OwnedSemaphorePermit,
}

impl TileImage {
    pub fn pixels(&self) -> &[u8] {
        self.image.as_bytes(0).unwrap()
    }
}

pub(super) enum RasterError {
    Invalid(String),
    Unavailable(String),
}

impl RasterError {
    pub fn message(self) -> String {
        match self {
            Self::Invalid(message) | Self::Unavailable(message) => message,
        }
    }
}

impl From<&str> for RasterError {
    fn from(message: &str) -> Self {
        Self::Invalid(message.into())
    }
}

impl From<png::DecodingError> for RasterError {
    fn from(error: png::DecodingError) -> Self {
        Self::Invalid(error.to_string())
    }
}

pub(super) fn decode(
    bytes: &[u8],
    reserve: impl FnOnce() -> Result<OwnedSemaphorePermit>,
) -> std::result::Result<Arc<TileImage>, RasterError> {
    let mut input = Cursor::new(bytes);
    let mut options = DecodeOptions::default();
    options.set_skip_ancillary_crc_failures(false);
    let mut decoder = Decoder::new_with_options(&mut input, options);
    decoder.set_limits(Limits { bytes: 1024 * 1024 });
    decoder.ignore_checksums(false);
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_transformations(Transformations::normalize_to_color8());
    let header = decoder.read_header_info()?;
    if (header.width, header.height) != (256, 256) {
        return Err("Expected a 256 by 256 tile".into());
    }
    let mut reader = decoder.read_info()?;
    if reader.info().animation_control.is_some() {
        return Err("Animated tiles are unsupported".into());
    }
    let reservation = reserve().map_err(RasterError::Unavailable)?;
    let size = reader
        .output_buffer_size()
        .filter(|n| *n <= TILE_BYTES)
        .ok_or("Tile raster exceeds its reservation")?;
    let mut samples = vec![0; size];
    let frame = reader.next_frame(&mut samples)?;
    reader.finish()?;
    drop(reader);
    if input.position() != bytes.len() as u64 {
        return Err("Trailing tile data".into());
    }
    let mut bgra = Vec::with_capacity(TILE_BYTES);
    for sample in samples.chunks_exact(frame.color_type.samples()) {
        bgra.extend_from_slice(&match frame.color_type {
            ColorType::Rgb => [sample[2], sample[1], sample[0], 255],
            ColorType::Rgba => [sample[2], sample[1], sample[0], sample[3]],
            ColorType::Grayscale => [sample[0], sample[0], sample[0], 255],
            ColorType::GrayscaleAlpha => [sample[0], sample[0], sample[0], sample[1]],
            ColorType::Indexed => return Err("Unexpanded palette in tile".into()),
        });
    }
    let rgba = image::RgbaImage::from_raw(256, 256, bgra).ok_or("Incomplete tile raster")?;
    Ok(Arc::new(TileImage {
        image: Arc::new(RenderImage::new([image::Frame::new(rgba)])),
        _reservation: reservation,
    }))
}

/// Owns the decoded reservation until the window retires its atlas entry.
pub struct TilePaintLease(Arc<TileImage>);

impl TilePaintLease {
    pub fn new(image: Arc<TileImage>) -> Self {
        Self(image)
    }

    pub fn matches(&self, image: &Arc<TileImage>) -> bool {
        Arc::ptr_eq(&self.0, image)
    }

    pub fn paint(
        &self,
        bounds: gpui_kit::Bounds<gpui_kit::Pixels>,
        window: &mut gpui_kit::Window,
    ) -> Result<()> {
        window
            .paint_image(
                bounds,
                bounds,
                Default::default(),
                self.0.image.clone(),
                0,
                false,
            )
            .map_err(|error| error.to_string())
    }

    pub fn retire(self, window: Option<&mut gpui_kit::Window>, cx: &mut gpui_kit::App) {
        cx.drop_image(self.0.image.clone(), window);
    }
}
