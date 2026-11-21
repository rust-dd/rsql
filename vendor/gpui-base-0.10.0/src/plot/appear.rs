//! The appear motion shared by every [`Plot`](super::Plot): how far its data
//! marks have drawn in since the plot was first painted.
//!
//! This is behavior only. A plot decides what appearing looks like — a line
//! revealed from the left, bars growing from zero — and a styled layer
//! projects the timing through [`PlotMotion`](crate::PlotMotion).
//!
//! The appear lives in the plot's element state, so a plot that stops being
//! painted — a row a virtual list scrolled away — forgets it and draws in again
//! when it comes back. A [`PlotAppearScope`] around the list remembers which
//! plots have finished, for as long as the scope itself is painted.
use std::{cell::RefCell, collections::HashMap, panic::Location, rc::Rc};

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window,
};

use crate::{
    Theme,
    motion::{Easing, Presence},
};

/// How far a plot's data marks have appeared this frame, handed to
/// [`Plot::appear`](super::Plot::appear).
///
/// The appear starts on the first frame a plot's id is painted and runs over
/// the active [`PlotMotion`](crate::PlotMotion)'s appear. Base's default
/// duration is zero, and reduced motion skips it, so a plot is then complete
/// from its first frame.
#[derive(Clone)]
pub struct PlotAppear {
    /// Linear time through the appear, from `0` to `1`.
    time: f32,
    easing: Easing,
}

impl PlotAppear {
    /// A finished appear: every mark is complete.
    pub fn complete() -> Self {
        Self {
            time: 1.,
            easing: Easing::Linear,
        }
    }

    /// How far the whole plot has appeared, from `0` to `1`, eased.
    pub fn progress(&self) -> f32 {
        // Charts read this per mark on every frame, long after the appear is
        // done, so a finished appear skips sampling the curve.
        if self.time >= 1. {
            return 1.;
        }
        self.easing.sample(self.time)
    }

    /// Whether the appear is still running.
    pub fn is_appearing(&self) -> bool {
        self.time < 1.
    }

    /// How far mark `index` of `count` has appeared, from `0` to `1`, eased.
    ///
    /// The marks start one after another across the first `spread` of the
    /// appear (`0..1`) and each runs for the rest of it, so the last mark
    /// still finishes with the appear however many marks there are. A `spread`
    /// of `0` moves every mark together.
    pub fn staggered(&self, index: usize, count: usize, spread: f32) -> f32 {
        if count <= 1 || self.time >= 1. {
            return self.progress();
        }
        let spread = spread.clamp(0., 0.95);
        let start = spread * index.min(count - 1) as f32 / (count - 1) as f32;
        let time = ((self.time - start) / (1. - spread)).clamp(0., 1.);
        self.easing.sample(time)
    }
}

/// The element-state key of a plot's appear, within the plot's scope.
const APPEAR: &str = "__plot-appear";

/// The plots a [`PlotAppearScope`] has seen finish appearing, by global id,
/// with the generation that finished.
type Appeared = Rc<RefCell<HashMap<GlobalElementId, u64>>>;

thread_local! {
    /// The scopes being laid out or painted, innermost last. Only non-empty
    /// while a [`PlotAppearScope`] is drawing its child.
    static SCOPES: RefCell<Vec<Appeared>> = const { RefCell::new(Vec::new()) };
}

/// Remembers which plots inside it have finished appearing, so a plot that is
/// painted again after a gap — a row a virtual list scrolled out of view and
/// back — shows its data whole instead of drawing in again.
///
/// Wrap the list, or whatever region repaints its plots on and off, in one:
///
/// ```ignore
/// PlotAppearScope::new(("transcript", conversation_id), list(state, render_row).flex_1())
/// ```
///
/// A plot is remembered by its global element id and its
/// [`Plot::appear_generation`](super::Plot::appear_generation), once its appear
/// has finished; a new generation still replays it, and one taken away
/// mid-appear draws in again from the start. The memory is the scope's own
/// element state: it lasts while the scope is painted every frame and goes with
/// it, so a view that is closed, or a scope whose id changes — name it after the
/// content, such as a conversation — draws its plots in afresh. The innermost
/// scope wins. Without one, every plot draws in each time it is painted anew.
///
/// The scope takes no part in layout: it hands on its child's layout, so a
/// child that sizes itself, such as a `list`, keeps doing so.
pub struct PlotAppearScope {
    id: ElementId,
    child: Option<AnyElement>,
}

impl PlotAppearScope {
    /// Remember the appears of the plots in `child` under `id`, unique among
    /// its siblings.
    pub fn new(id: impl Into<ElementId>, child: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            child: Some(child.into_any_element()),
        }
    }

    /// Run `f` with `appeared` as the innermost scope.
    fn within<R>(appeared: &Appeared, f: impl FnOnce() -> R) -> R {
        SCOPES.with_borrow_mut(|scopes| scopes.push(appeared.clone()));
        let result = f();
        SCOPES.with_borrow_mut(|scopes| scopes.pop());
        result
    }
}

#[cfg(not(feature = "gpui-fast"))]
impl IntoElement for PlotAppearScope {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for PlotAppearScope {
    type RequestLayoutState = (Option<AnyElement>, Appeared);
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let appeared: Appeared = match global_id {
            Some(global_id) => window.with_element_state(global_id, |appeared, _| {
                let appeared: Appeared = appeared.unwrap_or_default();
                (appeared.clone(), appeared)
            }),
            None => Appeared::default(),
        };
        let mut child = self.child.take();
        let layout_id = Self::within(&appeared, || match child.as_mut() {
            Some(child) => child.request_layout(window, cx),
            None => window.request_layout(Default::default(), None, cx),
        });
        (layout_id, (child, appeared))
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        (child, appeared): &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        // A virtual list lays its rows out here, so their plots appear here.
        if let Some(child) = child {
            Self::within(appeared, || {
                child.prepaint(window, cx);
            });
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        (child, appeared): &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(child) = child {
            Self::within(appeared, || child.paint(window, cx));
        }
    }
}

/// The innermost scope, if any, cloned out so no borrow outlives the lookup.
fn current_scope() -> Option<Appeared> {
    SCOPES.with_borrow(|scopes| scopes.last().cloned())
}

/// Sample the appear of the plot painting under the window's current element
/// id, `global_id`. A new `generation` starts it over, and one the innermost
/// [`PlotAppearScope`] saw finish is complete at once. Called by
/// [`PlotElement`](super::PlotElement) within the plot's element scope on every
/// frame, so it borrows the theme rather than cloning it and builds its key
/// without allocating.
pub(super) fn track_appear(
    global_id: &GlobalElementId,
    generation: u64,
    window: &mut Window,
    cx: &mut App,
) -> PlotAppear {
    let scope = current_scope();
    if scope
        .as_ref()
        .is_some_and(|scope| scope.borrow().get(global_id) == Some(&generation))
    {
        return PlotAppear::complete();
    }
    let Some(policy) = cx
        .try_global::<Theme>()
        .map(|theme| theme.plot.motion().appear().clone())
    else {
        return PlotAppear::complete();
    };
    let easing = policy.curve().clone();
    // Presence keeps the linear time so the marks can each ease over their
    // own slice of it; see `PlotAppear::staggered`.
    let sample = Presence::new((ElementId::Integer(generation), APPEAR), true)
        .transition(policy.easing(Easing::Linear))
        .sample(window, cx);
    if sample.progress >= 1.
        && let Some(scope) = scope
    {
        scope.borrow_mut().insert(global_id.clone(), generation);
    }
    PlotAppear {
        time: sample.progress,
        easing,
    }
}
