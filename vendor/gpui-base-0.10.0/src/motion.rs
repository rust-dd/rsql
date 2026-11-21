#[cfg(not(target_family = "wasm"))]
use std::time::Instant;
use std::{cell::RefCell, rc::Rc, time::Duration};
#[cfg(target_family = "wasm")]
use web_time::Instant;

use gpui::{
    App, Bounds, ElementId, Pixels, SharedString, Size, SpringConfig, SpringState, SpringTarget,
    Window,
};

use crate::animation::{Lerp, ease_out_cubic};

mod easing;
mod keyframes;
mod presence;
mod reveal;
mod sequence;
mod stagger;
mod timing;

pub use easing::{Easing, EasingError, LinearStop, StepPosition};
pub use keyframes::{Discrete, DiscreteError, Keyframe, KeyframeError, Keyframes};
pub use presence::{Presence, PresencePhase, PresenceSample};
pub use reveal::MotionReveal;
pub use sequence::{Sequence, SequenceSample, SequenceStep};
pub use stagger::{Stagger, StaggerOrigin};
pub use timing::{
    IterationCount, MotionPhase, PlaybackDirection, SignedDuration, Timing, TimingSample,
};

/// Matches GPUI's own default spring settling tolerance.
const DEFAULT_SPRING_EPSILON: f32 = 0.001;

/// A value that can be interpolated between two application-owned targets.
pub trait Interpolate: Clone {
    fn interpolate(&self, target: &Self, progress: f32) -> Self;
}

impl<T: Lerp> Interpolate for T {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        self.lerp(target, progress)
    }
}

impl Interpolate for Size<Pixels> {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        Size::new(
            self.width.lerp(&target.width, progress),
            self.height.lerp(&target.height, progress),
        )
    }
}

impl Interpolate for Bounds<Pixels> {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        Bounds::new(
            self.origin.lerp(&target.origin, progress),
            self.size.interpolate(&target.size, progress),
        )
    }
}

/// A presentation-neutral bundle for coordinated paint transforms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionTransform {
    pub translation: gpui::Point<Pixels>,
    pub scale: gpui::Point<f32>,
    pub rotation_radians: f32,
    pub opacity: f32,
}

impl MotionTransform {
    pub fn identity() -> Self {
        Self {
            translation: gpui::point(gpui::px(0.0), gpui::px(0.0)),
            scale: gpui::point(1.0, 1.0),
            rotation_radians: 0.0,
            opacity: 1.0,
        }
    }
}

impl Default for MotionTransform {
    fn default() -> Self {
        Self::identity()
    }
}

impl Interpolate for MotionTransform {
    fn interpolate(&self, target: &Self, progress: f32) -> Self {
        Self {
            translation: self.translation.lerp(&target.translation, progress),
            scale: gpui::point(
                self.scale.x.lerp(&target.scale.x, progress),
                self.scale.y.lerp(&target.scale.y, progress),
            ),
            rotation_radians: self
                .rotation_radians
                .lerp(&target.rotation_radians, progress),
            opacity: self.opacity.lerp(&target.opacity, progress),
        }
    }
}

/// How a value transition responds to the application's reduced-motion setting.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReducedMotion {
    /// Adopt the target immediately. Appropriate for spatial motion.
    #[default]
    Finish,
    /// Keep the transition timing, for non-spatial effects such as fades.
    Continue,
}

/// CSS-like timing policy for a target-value transition.
///
/// This type is intentionally separate from [`crate::animation::Transition`],
/// whose legacy interface applies concrete fade, slide, and size effects to an
/// element. A value transition never chooses a visual property for the caller.
#[derive(Clone)]
pub struct Transition {
    duration: Duration,
    delay: SignedDuration,
    easing: Easing,
    reduced_motion: ReducedMotion,
}

impl Transition {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            delay: SignedDuration::ZERO,
            easing: Easing::Custom(Rc::new(ease_out_cubic)),
            reduced_motion: ReducedMotion::Finish,
        }
    }

    /// Select reduced-motion behavior for this transition only.
    /// The application's preference and other motion channels are unchanged.
    ///
    /// [`transition`], [`transition_with_status`] and
    /// [`TransitionState::sample`] read it. [`Presence`] still finishes at
    /// once under reduced motion.
    pub fn reduced_motion(mut self, behavior: ReducedMotion) -> Self {
        self.reduced_motion = behavior;
        self
    }

    pub fn delay(mut self, delay: impl Into<SignedDuration>) -> Self {
        self.delay = delay.into();
        self
    }

    pub fn ease(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.easing = Easing::Custom(Rc::new(easing));
        self
    }

    pub fn easing(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    fn sample(&self, progress: f32) -> f32 {
        self.easing.sample(progress)
    }

    /// The easing curve, for a caller that samples one timeline at several
    /// offsets, such as a staggered plot appear.
    pub(crate) fn curve(&self) -> &Easing {
        &self.easing
    }

    fn progress(&self, elapsed: Duration, duration: Duration) -> (f32, MotionStatus) {
        let Some(active_elapsed) = self.delay.active_elapsed(elapsed) else {
            return (0.0, MotionStatus::Delayed);
        };
        if duration.is_zero() || active_elapsed >= duration {
            return (1.0, MotionStatus::Finished);
        }
        (
            active_elapsed.as_secs_f32() / duration.as_secs_f32(),
            MotionStatus::Running,
        )
    }

    /// The elapsed time at which [`Self::progress`] over the transition's own
    /// duration first reports `Finished`.
    fn finishes_after(&self) -> Duration {
        match self.delay {
            SignedDuration::Positive(delay) => delay.saturating_add(self.duration),
            SignedDuration::Negative(delay) => self.duration.saturating_sub(delay),
        }
    }
}

impl From<Duration> for SignedDuration {
    fn from(duration: Duration) -> Self {
        Self::positive(duration)
    }
}

/// Identifies one independently transitioning value.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TransitionId(ElementId);

impl From<ElementId> for TransitionId {
    fn from(id: ElementId) -> Self {
        Self(id)
    }
}

impl From<&'static str> for TransitionId {
    fn from(id: &'static str) -> Self {
        Self(id.into())
    }
}

impl From<String> for TransitionId {
    fn from(id: String) -> Self {
        Self(id.into())
    }
}

impl From<SharedString> for TransitionId {
    fn from(id: SharedString) -> Self {
        Self(id.into())
    }
}

impl From<usize> for TransitionId {
    fn from(id: usize) -> Self {
        Self(id.into())
    }
}

impl From<i32> for TransitionId {
    fn from(id: i32) -> Self {
        Self(id.into())
    }
}

impl From<TransitionId> for ElementId {
    fn from(id: TransitionId) -> Self {
        ElementId::NamedChild(id.0.into(), "__base-transition-state".into())
    }
}

impl<I, C> From<(I, C)> for TransitionId
where
    I: Into<ElementId>,
    C: Into<SharedString>,
{
    fn from((id, channel): (I, C)) -> Self {
        Self(ElementId::NamedChild(id.into().into(), channel.into()))
    }
}

#[derive(Clone)]
struct ValueTransition<T> {
    from: T,
    /// Where the running transition logically started, before any reversal
    /// shortened it: what a CSS transition calls its reversing-adjusted start.
    reversing_adjusted_start: T,
    /// The policy the running transition started with, kept until it is
    /// retargeted.
    policy: Transition,
    target: T,
    started_at: Instant,
    reversing_factor: f32,
    duration: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MotionStatus {
    Idle,
    Delayed,
    Running,
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionValue<T> {
    pub value: T,
    pub status: MotionStatus,
}

/// Caller-owned state for one target-value transition, independent of mounting.
///
/// Keyed state lives only while its element renders, so a virtualized row that
/// scrolls out and back restarts at its target. Keep a `TransitionState` with the
/// row's data instead and sample it while rendering:
///
/// ```ignore
/// let opacity = row.highlight.sample(target, policy, window, cx).value;
/// ```
///
/// Clones share its clock and interruption/reversal history. The first sample
/// adopts its target immediately; later samples use the same algorithm as
/// [`transition_with_status`]. No view or window is retained. A new handle
/// resets it.
#[derive(Clone)]
pub struct TransitionState<T> {
    state: Rc<RefCell<Option<ValueTransition<T>>>>,
}

impl<T> Default for TransitionState<T> {
    fn default() -> Self {
        Self {
            state: Rc::new(RefCell::new(None)),
        }
    }
}

impl<T: Interpolate + PartialEq + 'static> TransitionState<T> {
    /// Returns the current value toward `target`, as [`transition_with_status`]
    /// does, and requests the next animation frame while the status is
    /// `Delayed` or `Running`.
    ///
    /// Call this while rendering an element. A target that changes while the
    /// element is not rendered starts transitioning at the next sample.
    pub fn sample(
        &self,
        target: T,
        policy: Transition,
        window: &mut Window,
        cx: &mut App,
    ) -> MotionValue<T> {
        let now = cx.background_executor().now();
        let mut state = self.state.borrow_mut();
        let state = state.get_or_insert_with(|| ValueTransition {
            from: target.clone(),
            reversing_adjusted_start: target.clone(),
            policy: policy.clone(),
            target: target.clone(),
            started_at: now,
            reversing_factor: 1.0,
            duration: policy.duration,
        });
        let snapshot = state.clone();
        let sampled = sample_transition(
            &snapshot,
            target,
            policy,
            now,
            cx.reduce_motion(),
            |update| update(state),
        );
        if matches!(
            sampled.status,
            MotionStatus::Delayed | MotionStatus::Running
        ) {
            window.request_animation_frame();
        }
        sampled
    }
}

/// Returns the current value for a CSS-like transition toward `target`.
///
/// State is keyed by `id`. The first value is adopted immediately; later target
/// changes transition from the value sampled at that instant. Components opt
/// into this function explicitly—base components do not install default motion.
///
/// Call this while rendering an element, where GPUI keyed element state is
/// available. A channel id must identify one value type within that element.
pub fn transition<T>(
    id: impl Into<TransitionId>,
    target: T,
    policy: Transition,
    window: &mut Window,
    cx: &mut App,
) -> T
where
    T: Interpolate + PartialEq + 'static,
{
    transition_with_status(id, target, policy, window, cx).value
}

pub fn transition_with_status<T>(
    id: impl Into<TransitionId>,
    target: T,
    policy: Transition,
    window: &mut Window,
    cx: &mut App,
) -> MotionValue<T>
where
    T: Interpolate + PartialEq + 'static,
{
    let id: ElementId = id.into().into();
    let now = cx.background_executor().now();
    let state = window.use_keyed_state(id, cx, |_, _| ValueTransition {
        from: target.clone(),
        reversing_adjusted_start: target.clone(),
        policy: policy.clone(),
        target: target.clone(),
        started_at: now,
        reversing_factor: 1.0,
        duration: policy.duration,
    });

    let snapshot = state.read(cx).clone();
    let reduced = cx.reduce_motion();
    let sampled = sample_transition(&snapshot, target, policy, now, reduced, |update| {
        state.update(cx, |state, _| update(state));
    });
    if matches!(
        sampled.status,
        MotionStatus::Delayed | MotionStatus::Running
    ) {
        window.request_animation_frame();
    }
    sampled
}

fn sample_transition<T>(
    snapshot: &ValueTransition<T>,
    target: T,
    policy: Transition,
    now: Instant,
    reduced_motion: bool,
    mut update: impl FnMut(&mut dyn FnMut(&mut ValueTransition<T>)),
) -> MotionValue<T>
where
    T: Interpolate + PartialEq + 'static,
{
    if (reduced_motion && policy.reduced_motion == ReducedMotion::Finish)
        || policy.duration.is_zero()
    {
        if snapshot.from != target || snapshot.target != target {
            update(&mut |state| {
                state.from = target.clone();
                state.reversing_adjusted_start = target.clone();
                state.policy = policy.clone();
                state.target = target.clone();
                state.started_at = now;
                state.reversing_factor = 1.0;
                state.duration = policy.duration;
            });
        }
        return MotionValue {
            value: target,
            status: MotionStatus::Finished,
        };
    }

    let elapsed = now.saturating_duration_since(snapshot.started_at);
    let (progress, status) = snapshot.policy.progress(elapsed, snapshot.duration);
    let sampled = snapshot
        .from
        .interpolate(&snapshot.target, snapshot.policy.sample(progress));

    let (value, status) = if snapshot.target != target {
        // As CSS transitions do: only a running transition sent back to where
        // it logically started is a reversal.
        let reversing = matches!(status, MotionStatus::Running | MotionStatus::Delayed)
            && target == snapshot.reversing_adjusted_start;
        let reversing_factor = if reversing {
            (snapshot.policy.sample(progress) * snapshot.reversing_factor
                + (1.0 - snapshot.reversing_factor))
                .abs()
                .clamp(0.0, 1.0)
        } else {
            1.0
        };
        let duration = policy.duration.mul_f64(f64::from(reversing_factor));
        // A reversal shortens a negative delay by the same factor.
        let mut active_policy = policy.clone();
        if reversing && let SignedDuration::Negative(delay) = policy.delay {
            active_policy.delay =
                SignedDuration::negative(delay.mul_f64(f64::from(reversing_factor)));
        }
        update(&mut |state| {
            state.reversing_adjusted_start = if reversing {
                snapshot.target.clone()
            } else {
                sampled.clone()
            };
            state.policy = active_policy.clone();
            state.from = sampled.clone();
            state.target = target.clone();
            state.started_at = now;
            state.reversing_factor = reversing_factor;
            state.duration = duration;
        });
        let (initial_progress, initial_status) = active_policy.progress(Duration::ZERO, duration);
        (
            sampled.interpolate(&target, active_policy.sample(initial_progress)),
            initial_status,
        )
    } else {
        (
            sampled,
            if snapshot.from == snapshot.target {
                MotionStatus::Idle
            } else {
                status
            },
        )
    };
    MotionValue { value, status }
}

#[derive(Clone, Copy)]
struct KeyframePlayback {
    started_at: Instant,
}

/// Samples a keyed keyframe playback and requests frames while it is active.
///
/// The stable `id` owns the playback's start time. Re-rendering with the same
/// ID continues that playback; it does not restart when `keyframes` or `timing`
/// is reconstructed. To replay a sequence, include an application-owned
/// generation in the ID, for example `("notification-enter", generation)`.
pub fn animate_keyframes<T>(
    id: impl Into<TransitionId>,
    keyframes: &Keyframes<T>,
    timing: Timing,
    window: &mut Window,
    cx: &mut App,
) -> MotionValue<T>
where
    T: Interpolate + 'static,
{
    let id: TransitionId = id.into();
    let id = ElementId::NamedChild(ElementId::from(id).into(), "__keyframes".into());
    let now = cx.background_executor().now();
    let state = window.use_keyed_state(id, cx, |_, _| KeyframePlayback { started_at: now });
    let started_at = state.read(cx).started_at;

    if cx.reduce_motion() {
        return MotionValue {
            value: keyframes.sample(1.0),
            status: MotionStatus::Finished,
        };
    }

    let sample = timing.sample(now.saturating_duration_since(started_at));
    let status = match sample.phase {
        MotionPhase::Before => MotionStatus::Delayed,
        MotionPhase::Active => MotionStatus::Running,
        MotionPhase::After => MotionStatus::Finished,
    };
    if matches!(status, MotionStatus::Delayed | MotionStatus::Running) {
        window.request_animation_frame();
    }
    MotionValue {
        value: keyframes.sample(sample.directed_progress),
        status,
    }
}

/// A physical spring policy for [`spring`].
///
/// A spring is the counterpart to [`Transition`] for values that can be
/// retargeted while they are still moving. A duration-based transition restarts
/// its easing from the value sampled at that instant, which is continuous in
/// position but not in velocity. A spring carries velocity across the retarget,
/// so a value reversed mid-flight decelerates and turns around instead of
/// snapping to a new curve's initial speed.
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    response: Duration,
    damping: f32,
    epsilon: f32,
    travel: bool,
}

/// Invalid physical or settling parameters for a [`Spring`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpringError {
    InvalidDamping,
    InvalidEpsilon,
}

impl std::fmt::Display for SpringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDamping => f.write_str("spring damping must be finite and non-negative"),
            Self::InvalidEpsilon => {
                f.write_str("spring epsilon must be finite and greater than zero")
            }
        }
    }
}

impl std::error::Error for SpringError {}

impl Spring {
    /// Builds a spring that reaches its target in about `response` without
    /// overshooting it.
    ///
    /// `response` is not a duration in the sense [`Transition::new`] means one.
    /// A spring has no end to schedule: this is the period one full oscillation
    /// would take without damping, which is the scale the motion is felt at
    /// rather than the moment it stops. The remaining fraction of a percent
    /// keeps settling past it, until it is within the tolerance
    /// [`Self::with_epsilon`] sets.
    ///
    /// A zero response adopts the target on the spot, as a zero duration does
    /// for a transition. Say that with [`Self::with_travel`] where it is what
    /// you mean; a zero here is the degenerate case, defined so an infinitely
    /// stiff spring resolves rather than dividing by its own period.
    pub const fn new(response: Duration) -> Self {
        Self {
            response,
            damping: 1.0,
            epsilon: DEFAULT_SPRING_EPSILON,
            travel: true,
        }
    }

    /// Sets the damping ratio, which is `1.0` — no overshoot — by default.
    ///
    /// Below `1.0` the spring passes its target and comes back; above `1.0` it
    /// approaches slowly. Overshoot suits a value with room to pass its target
    /// and nothing to collide with. A height, an opacity, or anything bounded by
    /// the geometry around it should stay at the default.
    ///
    /// This is $\zeta$, not GPUI's `SpringConfig::damping`, which is the
    /// coefficient $c = 2 \zeta \omega_0$.
    ///
    /// # Panics
    ///
    /// Panics when `ratio` is negative or non-finite. Use
    /// [`Self::try_with_damping`] when the value is not a trusted constant.
    pub const fn with_damping(self, ratio: f32) -> Self {
        match self.try_with_damping(ratio) {
            Ok(spring) => spring,
            Err(_) => panic!("spring damping must be finite and non-negative"),
        }
    }

    /// Checked form of [`Self::with_damping`].
    pub const fn try_with_damping(mut self, ratio: f32) -> Result<Self, SpringError> {
        if !ratio.is_finite() || ratio < 0.0 {
            return Err(SpringError::InvalidDamping);
        }
        self.damping = ratio;
        Ok(self)
    }

    /// Sets whether the spring travels to its target or adopts it on the spot.
    ///
    /// A value the pointer is already moving — a panel being dragged by its
    /// resize handle — must not lag behind the pointer, so the spring stops
    /// travelling for as long as the drag lasts. Retained state stays pinned to
    /// the target meanwhile, so travel resumes from the value the drag released
    /// rather than from wherever the spring was when it began.
    ///
    /// This says at the call that the motion is suspended, and it says it
    /// without disturbing the response, damping or tolerance the spring is
    /// configured with — which a policy swapped out for the length of the drag
    /// would have to restate or discard.
    pub const fn with_travel(mut self, travel: bool) -> Self {
        self.travel = travel;
        self
    }

    /// Sets the settling tolerance, expressed in the target's own units.
    ///
    /// The default suits targets that move within a normalized `0..1` range. A
    /// spring over pixels settles perceptibly sooner with a coarser tolerance,
    /// which also ends the animation frames that the remaining sub-pixel motion
    /// would otherwise request.
    ///
    /// # Panics
    ///
    /// Panics when `epsilon` is zero, negative, or non-finite. Use
    /// [`Self::try_with_epsilon`] when the value is not a trusted constant.
    pub const fn with_epsilon(self, epsilon: f32) -> Self {
        match self.try_with_epsilon(epsilon) {
            Ok(spring) => spring,
            Err(_) => panic!("spring epsilon must be finite and greater than zero"),
        }
    }

    /// Checked form of [`Self::with_epsilon`].
    pub const fn try_with_epsilon(mut self, epsilon: f32) -> Result<Self, SpringError> {
        if !epsilon.is_finite() || epsilon <= 0.0 {
            return Err(SpringError::InvalidEpsilon);
        }
        self.epsilon = epsilon;
        Ok(self)
    }

    /// Returns the settling tolerance in the target's own units.
    pub const fn epsilon(self) -> f32 {
        self.epsilon
    }

    /// The physical parameters GPUI integrates. The response must be non-zero;
    /// [`spring`] adopts the target before reaching here when it is not.
    ///
    /// Derived on use rather than stored, so the builders stay `const`: neither
    /// `Duration::as_secs_f32` nor the square root that recovers a damping ratio
    /// from a built config can be called from a `const fn`.
    fn config(&self) -> SpringConfig {
        let frequency = std::f32::consts::TAU / self.response.as_secs_f32();
        SpringConfig::new(frequency * frequency, 2.0 * self.damping * frequency, 1.0)
    }
}

#[derive(Clone, Copy)]
struct SpringTransition {
    state: SpringState,
    target: f32,
    updated_at: Instant,
}

/// Returns the current value for a spring travelling toward `target`.
///
/// State is keyed by `id` exactly as [`transition`] keys its own. The first
/// value is adopted immediately; later target changes preserve both the current
/// position and the current velocity, so an interrupted spring is redirected
/// rather than restarted.
///
/// Call this while rendering an element, where GPUI keyed element state is
/// available. A channel id must identify one value within that element.
pub fn spring<T>(
    id: impl Into<TransitionId>,
    target: T,
    policy: Spring,
    window: &mut Window,
    cx: &mut App,
) -> T::Output
where
    T: SpringTarget,
{
    let id: ElementId = id.into().into();
    let now = cx.background_executor().now();
    let target_position = target.target();
    let state = window.use_keyed_state(id, cx, |_, _| SpringTransition {
        state: SpringState {
            position: target_position,
            velocity: 0.0,
        },
        target: target_position,
        updated_at: now,
    });

    let snapshot = *state.read(cx);
    let at_rest_on_target =
        snapshot.state.position == target_position && snapshot.state.velocity == 0.0;

    // The overwhelmingly common case: a spring nothing is currently moving. It
    // has no state to advance and no frame to ask for, so it never builds a
    // config or steps one — a settled spring costs a read and two comparisons.
    // Every branch below would return this same value and write nothing.
    //
    // Resting writes nothing, so `updated_at` goes stale for as long as the rest
    // lasts. The next retarget then steps a zero displacement at zero velocity
    // over that whole gap, which any elapsed time leaves where it is, so the
    // stale clock cannot move the value — it only has to not produce a NaN, and
    // every term the propagator scales is finite.
    if at_rest_on_target {
        return target.resolve(target_position);
    }

    let settle = |state: &mut SpringTransition| {
        state.state = SpringState {
            position: target_position,
            velocity: 0.0,
        };
        state.target = target_position;
        state.updated_at = now;
    };

    if cx.reduce_motion() || !policy.travel || policy.response.is_zero() {
        state.update(cx, |state, _| settle(state));
        return target.resolve(target_position);
    }

    // Advance over the frame that just elapsed, which the previous target
    // governed, before adopting the new one for the frame to come.
    let elapsed = now
        .saturating_duration_since(snapshot.updated_at)
        .as_secs_f32();
    let config = policy.config();
    let stepped = config.step(snapshot.state, snapshot.target, elapsed);

    if config.is_settled(stepped, target_position, policy.epsilon) {
        state.update(cx, |state, _| settle(state));
        return target.resolve(target_position);
    }

    state.update(cx, |state, _| {
        state.state = stepped;
        state.target = target_position;
        state.updated_at = now;
    });
    window.request_animation_frame();
    target.resolve(stepped.position)
}
