use gpui_kit::component::input::{Copy, Cut};
use gpui_kit::{App, Global, InteractiveElement};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Default)]
struct ClipboardOrder(Arc<AtomicU64>);
impl Global for ClipboardOrder {}

/// Start an app-owned clipboard write when the user requests it, before async preparation.
#[derive(Clone)]
pub struct ClipboardWrite {
    sequence: Arc<AtomicU64>,
    issued: u64,
}

impl ClipboardWrite {
    pub fn begin(cx: &mut App) -> Self {
        let sequence = cx.default_global::<ClipboardOrder>().0.clone();
        let issued = sequence.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        Self { sequence, issued }
    }
    pub fn current(&self) -> bool {
        self.sequence.load(Ordering::Relaxed) == self.issued
    }
}

/// Capture before a focused text input consumes a shortcut or a menu-dispatched action.
pub fn capture<T: InteractiveElement>(element: T) -> T {
    element
        .capture_action(|_: &Copy, _, cx| {
            ClipboardWrite::begin(cx);
            cx.propagate();
        })
        .capture_action(|_: &Cut, _, cx| {
            ClipboardWrite::begin(cx);
            cx.propagate();
        })
}
