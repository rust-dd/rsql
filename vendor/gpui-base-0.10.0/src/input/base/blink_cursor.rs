use gpui::{App, Context, Pixels, Subscription, Task, px};
use instant::Duration;
use web_time::Instant;

static INTERVAL: Duration = Duration::from_millis(500);
static PAUSE_DELAY: Duration = Duration::from_millis(300);

pub(super) const CURSOR_WIDTH: Pixels = px(1.);

/// Application-wide caret visibility policy.
///
/// The default blinks every 500 ms without an idle timeout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorBlink {
    interval: Duration,
    idle_timeout: Option<Duration>,
}

impl CursorBlink {
    /// Keep the focused caret visible without a timer.
    pub const fn steady() -> Self {
        Self {
            interval: Duration::ZERO,
            idle_timeout: None,
        }
    }

    /// Alternate visible and hidden phases of `interval` each; zero means steady.
    pub const fn blinking(interval: Duration) -> Self {
        Self {
            interval,
            idle_timeout: None,
        }
    }

    /// Stop blinking, with the caret visible, once `timeout` has passed since
    /// the last focus, edit or cursor movement.
    pub const fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = Some(timeout);
        self
    }

    /// The duration of each visible or hidden phase, or `None` when steady.
    pub fn interval(&self) -> Option<Duration> {
        (!self.interval.is_zero()).then_some(self.interval)
    }

    /// The time since the last activity after which blinking stops.
    pub fn idle_timeout(&self) -> Option<Duration> {
        self.interval().and(self.idle_timeout)
    }

    /// Whether the focused caret stays visible without blinking.
    pub fn is_steady(&self) -> bool {
        self.interval.is_zero()
    }

    /// Collapse every steady spelling to one value, so a change between them
    /// is not mistaken for a policy change.
    fn normalized(self) -> Self {
        if self.is_steady() {
            Self::steady()
        } else {
            self
        }
    }
}

impl Default for CursorBlink {
    fn default() -> Self {
        Self::blinking(INTERVAL)
    }
}

fn cursor_blink(cx: &App) -> CursorBlink {
    cx.try_global::<crate::Theme>()
        .map(|theme| theme.cursor_blink)
        .unwrap_or_default()
        .normalized()
}

pub(crate) struct BlinkCursor {
    visible: bool,
    paused: bool,
    epoch: usize,
    last_activity: Option<Instant>,
    policy: CursorBlink,
    _task: Task<()>,
    _policy_subscription: Subscription,
}

impl BlinkCursor {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            visible: false,
            paused: false,
            epoch: 0,
            last_activity: None,
            policy: cursor_blink(cx),
            _task: Task::ready(()),
            _policy_subscription: cx.observe_global::<crate::Theme>(|this, cx| {
                let policy = cursor_blink(cx);
                if policy == this.policy {
                    return;
                }
                this.policy = policy;
                if this.last_activity.is_some() {
                    this.epoch = this.epoch.wrapping_add(1);
                    this.set_visible(true, cx);
                    this.schedule(cx);
                }
            }),
        }
    }

    pub(crate) fn start(&mut self, cx: &mut Context<Self>) {
        self.epoch = self.epoch.wrapping_add(1);
        self.last_activity = Some(cx.background_executor().now());
        self.paused = false;
        self.set_visible(true, cx);
        self.schedule(cx);
    }

    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        self.epoch = self.epoch.wrapping_add(1);
        self.last_activity = None;
        self.paused = false;
        self._task = Task::ready(());
        self.set_visible(false, cx);
    }

    pub(crate) fn pause(&mut self, cx: &mut Context<Self>) {
        if self.last_activity.is_none() {
            return;
        }

        self.epoch = self.epoch.wrapping_add(1);
        self.last_activity = Some(cx.background_executor().now());
        self.paused = true;
        self.set_visible(true, cx);
        self.schedule(cx);
    }

    pub(crate) fn visible(&self) -> bool {
        self.visible
    }

    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible != visible {
            self.visible = visible;
            cx.notify();
        }
    }

    fn blink_delay(&self, cx: &App) -> Option<Duration> {
        let last_activity = self.last_activity?;
        let interval = self.policy.interval()?;

        let elapsed = cx.background_executor().now() - last_activity;
        let delay = if self.paused && elapsed < PAUSE_DELAY {
            PAUSE_DELAY - elapsed
        } else {
            interval
        };

        match self.policy.idle_timeout() {
            Some(timeout) if elapsed >= timeout => None,
            Some(timeout) => Some(delay.min(timeout - elapsed)),
            None => Some(delay),
        }
    }

    fn schedule(&mut self, cx: &mut Context<Self>) {
        self._task = Task::ready(());
        let Some(delay) = self.blink_delay(cx) else {
            self.paused = false;
            self.set_visible(true, cx);
            return;
        };

        let epoch = self.epoch;
        let timer = cx.background_executor().timer(delay);
        self._task = cx.spawn(async move |this, cx| {
            timer.await;
            _ = this.update(cx, |this, cx| {
                if epoch != this.epoch {
                    return;
                }

                if this.blink_delay(cx).is_some() {
                    this.paused = false;
                    this.set_visible(!this.visible, cx);
                }
                this.schedule(cx);
            });
        });
    }
}
