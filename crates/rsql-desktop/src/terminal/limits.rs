/// Maximum OSC payload, including parameter separators, before dispatch.
pub const MAX_OSC_BYTES: usize = 16 * 1024;
/// Maximum additional zero-width Unicode scalars stored with one cell.
pub const MAX_COMBINING_SCALARS: usize = 64;
/// Newly admitted input per call; this does not bound atomic sync decoding.
/// The owner also counts `processed` across calls and yields at this turn budget,
/// including when the final call returns Ready with no unprocessed suffix.
pub const MAX_FEED_BYTES: usize = 64 * 1024;
/// Pinned library synchronized-update buffer ceiling, independent of admission.
pub const MAX_SYNCHRONIZED_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalLimits {
    pub osc_bytes: usize,
    pub combining_scalars: usize,
}

impl Default for TerminalLimits {
    fn default() -> Self {
        Self {
            osc_bytes: MAX_OSC_BYTES,
            combining_scalars: MAX_COMBINING_SCALARS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputLimit {
    OscBytes { limit: usize },
    CombiningScalars { limit: usize },
}
