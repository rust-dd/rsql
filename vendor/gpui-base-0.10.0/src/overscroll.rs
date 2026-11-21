//! Boundary displacement shared by inputs and scroll viewports.
//!
//! Viewport owners keep their logical scroll position clamped and apply this
//! displacement only to content geometry. Inputs drive it directly, while
//! `ScrollBounce` can own a list or scroll area's vertical wheel handling.

use crate::scroll_bounce::Physics;
use gpui::TouchPhase;
use std::time::Duration;
#[cfg(not(target_family = "wasm"))]
use std::time::Instant;
#[cfg(target_family = "wasm")]
use web_time::Instant;

/// A pause without packets that ends a gesture.
///
/// Momentum arrives once per frame with no phase of its own, and some precise
/// scrolling devices send no phases, so a pause marks the end of their stream.
/// `ScrollBounce` waits the same time for the same reason.
const GESTURE_GAP: Duration = Duration::from_millis(250);

/// The fraction of the text's speed the edge carries when the text runs into
/// it or a finger lets go of it. Measured on `NSScrollView`, and the same as
/// WebKit's rubber-band amplitude: a fling arriving at 5000 px/s throws the
/// edge about 45 px out, 80 ms before it turns back.
const CARRY: f32 = 0.31;

/// A finger that sends nothing for this long before lifting has stopped:
/// it lets go of the edge with no speed.
const STILL: Duration = Duration::from_millis(50);

/// The interval assumed for the first packet of a stream.
const FRAME: f32 = 1. / 60.;

/// How far back the stream's speed is read. Packets carry no timestamp, so a
/// frame that takes long hands over several at once; read over a span, their
/// distance still spreads over the time they took, instead of each one
/// arriving at once and reading as a huge speed. That happens most on a fling
/// through a long list, just as it runs into the top.
const SPEED_SPAN: Duration = Duration::from_millis(50);

/// The most a packet's unused part can be and still be rounding. Far from
/// its start an `f32` offset cannot hold every fraction, so text that used a
/// whole packet can report a hair more or less, which is not an edge.
const ROUNDING: f32 = 0.01;

#[derive(Default)]
pub(crate) struct Overscroll {
    physics: Physics,
    sampled_at: Option<Instant>,
    last_packet_at: Option<Instant>,
    /// When the stream last moved, and how fast, in pixels per second.
    moved_at: Option<Instant>,
    velocity: f32,
    /// The packets that moved within the last `SPEED_SPAN`, and the one
    /// before them, which marks where the span starts.
    moves: Vec<(Instant, f32)>,
    /// A finger rests on the trackpad but has not moved since it touched.
    resting: bool,
    /// The edge already answered the rest of this gesture: a fling ran into
    /// it, or a finger let go of it stretched. The momentum that follows
    /// still scrolls the text back from the edge, but no longer moves the
    /// edge.
    spent: bool,
    /// The trackpad reported a touch or a lift in this stream, so its
    /// phaseless packets are momentum. A smooth-scrolling mouse sends the
    /// same packets on macOS, but never a touch or a lift.
    touched: bool,
}

impl Overscroll {
    pub(crate) fn set_motion(&mut self, motion: crate::ScrollBounceMotion) {
        self.physics.set_motion(motion);
    }
    /// How far the text is drawn from where it scrolled to.
    pub(crate) fn offset(&self) -> f32 {
        self.physics.offset()
    }

    /// A new gesture starts: it may stretch the edge again.
    fn start_gesture(&mut self) {
        self.spent = false;
        self.touched = false;
        self.velocity = 0.;
        self.moves.clear();
    }

    /// Track the stream's speed from packets that moved: the distance they
    /// covered over the time since the span started.
    fn sample(&mut self, dy: f32, now: Instant) {
        if dy == 0. {
            return;
        }
        if self.moves.last().is_some_and(|&(_, last)| last * dy < 0.) {
            self.moves.clear();
        }
        self.moves.push((now, dy));
        let start = self
            .moves
            .iter()
            .rposition(|&(at, _)| now.saturating_duration_since(at) > SPEED_SPAN)
            .unwrap_or(0);
        self.moves.drain(..start);
        let (first, _) = self.moves[0];
        let seconds = now.saturating_duration_since(first).as_secs_f32();
        self.velocity = if seconds > 0. && self.moves.len() > 1 {
            self.moves[1..].iter().map(|&(_, dy)| dy).sum::<f32>() / seconds
        } else {
            // The first packet, or several handed over at once.
            self.moves.iter().map(|&(_, dy)| dy).sum::<f32>() / FRAME
        };
        self.moved_at = Some(now);
    }

    /// Note a packet not handed to [`Self::packet`], such as a sideways one:
    /// it keeps the stream going, and a touch or a lift there still marks the
    /// stream as the trackpad's.
    pub(crate) fn touch_phase(&mut self, phase: TouchPhase, now: Instant) {
        self.follow(now);
        if phase != TouchPhase::Moved {
            self.touched = true;
        }
    }

    /// Start a new gesture after a pause, unless a finger holds the edge.
    fn follow(&mut self, now: Instant) {
        let gap = self
            .last_packet_at
            .replace(now)
            .map(|at| now.saturating_duration_since(at))
            .filter(|gap| *gap < GESTURE_GAP);
        // A finger resting on the trackpad sends nothing, however long it
        // holds the edge; only its own `Ended` lets go. A pause ends the
        // other streams.
        if gap.is_none() && !self.physics.is_dragging() {
            self.start_gesture();
            self.moved_at = None;
        }
    }

    /// Handle one vertical packet of `dy` over a viewport `extent` tall.
    /// `scroll` moves the text by a delta and returns how much of it the text
    /// used. Returns whether the stretch changed.
    pub(crate) fn packet(
        &mut self,
        dy: f32,
        phase: TouchPhase,
        extent: f32,
        now: Instant,
        mut scroll: impl FnMut(f32) -> f32,
    ) -> bool {
        let before = self.offset();
        let mut thrown = false;
        self.follow(now);
        self.sample(dy, now);
        if phase != TouchPhase::Moved {
            self.touched = true;
        }
        if phase == TouchPhase::Started {
            // Grab the edge, a returning one included, where it is.
            self.physics.begin(extent);
            self.spent = false;
            self.resting = dy == 0.;
        } else if dy != 0. {
            self.resting = false;
        } else if self.resting && self.physics.is_dragging() {
            // A finger that touches and lifts without moving cancels its
            // gesture, and GPUI reports the cancel as a move of nothing: it
            // lets go of the edge where it held it.
            self.resting = false;
            self.physics.let_go(0.);
            self.spent = self.offset() != 0.;
            if self.spent {
                self.sampled_at = Some(now);
            }
            return self.physics.is_moving();
        }
        if self.physics.is_dragging() {
            // A stretched edge takes the packet first; only what crosses back
            // over the edge scrolls the text.
            let rest = if self.offset() != 0. {
                self.physics.pull(dy)
            } else {
                dy
            };
            if rest != 0. {
                let unused = rest - scroll(rest);
                if unused.abs() > ROUNDING {
                    self.physics.pull(unused);
                }
            }
            if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                let moving = self
                    .moved_at
                    .is_some_and(|at| now.saturating_duration_since(at) < STILL);
                // Only an edge held past its end carries the finger's speed.
                // Let go within the text, the momentum that follows scrolls
                // it, and throws the edge only if it runs into it.
                let velocity = if moving && self.offset() != 0. {
                    self.velocity
                } else {
                    0.
                };
                self.physics.let_go(velocity * CARRY);
                thrown = true;
                self.spent = self.offset() != 0.;
            }
        } else if self.spent || cfg!(target_os = "macos") && !self.touched {
            // Momentum after the edge answered: back into the text it
            // scrolls; against the edge it has nothing left to move. A
            // smooth-scrolling mouse stops at the edge, as in NSScrollView.
            scroll(dy);
        } else {
            // Momentum or phaseless precise scrolling: it scrolls the text,
            // and when the text runs into the edge it throws the edge as
            // fast as it arrived, once.
            let unused = dy - scroll(dy);
            if unused.abs() > ROUNDING {
                self.physics.let_go(self.velocity * CARRY);
                thrown = true;
                self.spent = true;
            }
        }
        let changed = (thrown && self.physics.is_moving()) || self.offset() != before;
        // Start the clock when the edge starts moving, and again when it is
        // let go, however long it was held; a packet that arrives while it
        // moves must not lose the time since the last frame.
        if thrown || (changed && self.sampled_at.is_none()) {
            self.sampled_at = Some(now);
        }
        changed
    }

    /// Advance the return to `now`. Returns whether it is still moving.
    pub(crate) fn step(&mut self, now: Instant) -> bool {
        let elapsed = self
            .sampled_at
            .replace(now)
            .map_or(0., |at| now.saturating_duration_since(at).as_secs_f32());
        let moving = self.physics.step(elapsed);
        if !moving {
            self.sampled_at = None;
        }
        moving
    }

    /// Drop any stretch at once, as when bouncing is turned off.
    pub(crate) fn reset(&mut self) {
        let mut physics = std::mem::take(&mut self.physics);
        physics.reset();
        *self = Self {
            physics,
            ..Self::default()
        };
    }
}
