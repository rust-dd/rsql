use super::*;
use gpui_kit::component::{WindowExt as _, notification::Notification};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    fmt::Display,
};

const LIFETIME: Duration = Duration::from_millis(4_000);
const TICK: Duration = Duration::from_millis(100);
const ICONS: [(&str, &str); 2] = [
    (
        "icons/rsql-toast-success.svg",
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" height="20" width="20"><path fill-rule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.857-9.809a.75.75 0 00-1.214-.882l-3.483 4.79-1.88-1.88a.75.75 0 10-1.06 1.061l2.5 2.5a.75.75 0 001.137-.089l4-5.5z" clip-rule="evenodd"/></svg>"#,
    ),
    (
        "icons/rsql-toast-error.svg",
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" height="20" width="20"><path fill-rule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-8-5a.75.75 0 01.75.75v4.5a.75.75 0 01-1.5 0v-4.5A.75.75 0 0110 5zm0 10a1 1 0 100-2 1 1 0 000 2z" clip-rule="evenodd"/></svg>"#,
    ),
];

pub(super) fn icon(path: &str) -> Option<&'static [u8]> {
    ICONS
        .iter()
        .find(|(known, _)| *known == path)
        .map(|(_, svg)| svg.as_bytes())
}

#[derive(Clone, Copy)]
enum Tone {
    Success,
    Error,
}

impl Tone {
    /// Sonner's rich dark palette: the original Toaster is dark in both app themes.
    fn colors(self) -> [Hsla; 3] {
        let color = |h: f32, s: f32, l: f32| hsla(h / 360., s, l, 1.);
        match self {
            Self::Success => [
                color(150., 1., 0.06),
                color(147., 1., 0.12),
                color(150., 0.86, 0.65),
            ],
            Self::Error => [
                color(358., 0.76, 0.1),
                color(357., 0.89, 0.16),
                color(358., 1., 0.81),
            ],
        }
    }

    fn icon(self) -> &'static str {
        ICONS[self as usize].0
    }
}

pub(super) struct Toast {
    tone: Tone,
    title: SharedString,
    description: Option<SharedString>,
    lifetime: Duration,
}

impl Toast {
    pub(super) fn success(title: impl Display) -> Self {
        Self::new(Tone::Success, title)
    }

    pub(super) fn error(title: impl Display) -> Self {
        Self::new(Tone::Error, title)
    }

    fn new(tone: Tone, title: impl Display) -> Self {
        Self {
            tone,
            title: diagnostic::bounded_message(title).into(),
            description: None,
            lifetime: LIFETIME,
        }
    }

    pub(super) fn description(mut self, text: impl Display) -> Self {
        self.description = Some(diagnostic::bounded_message(text).into());
        self
    }

    pub(super) fn lifetime(mut self, lifetime: Duration) -> Self {
        self.lifetime = lifetime;
        self
    }

    fn into_notification(self, serial: u64, state: &Rc<State>) -> Notification {
        let [background, border, text] = self.tone.colors();
        let Self {
            tone,
            title,
            description,
            ..
        } = self;
        let label = match &description {
            Some(description) => format!("{title}. {description}").into(),
            None => title.clone(),
        };
        let (hover, close) = (state.clone(), state.clone());
        Notification::new()
            .id1::<ToastKey>(key(serial))
            .autohide(false)
            .p_0()
            .rounded(px(8.))
            .border_color(border)
            .bg(background)
            .on_close(move |_, _| {
                close.hovered.borrow_mut().remove(&serial);
            })
            .content(move |_, _, _| {
                let hover = hover.clone();
                div()
                    .id(key(serial))
                    .role(Role::Status)
                    .aria_label(label.clone())
                    .on_hover(move |hovered, _, _| {
                        let mut set = hover.hovered.borrow_mut();
                        if *hovered {
                            set.insert(serial);
                        } else {
                            set.remove(&serial);
                        }
                    })
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .p(px(16.))
                    .text_size(px(13.))
                    .text_color(text)
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .size(px(16.))
                            .ml(px(-3.))
                            .mr(px(4.))
                            .child(
                                svg()
                                    .path(tone.icon())
                                    .flex_none()
                                    .size(px(20.))
                                    .ml(px(-1.))
                                    .text_color(text),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .line_height(relative(1.5))
                                    .child(title.clone()),
                            )
                            .children(
                                description
                                    .clone()
                                    .map(|text| div().line_height(relative(1.4)).child(text)),
                            ),
                    )
                    .into_any_element()
            })
    }
}

struct ToastKey;

fn key(serial: u64) -> ElementId {
    ElementId::NamedInteger("rsql-toast".into(), serial)
}

#[derive(Default)]
struct State {
    serial: Cell<u64>,
    hovered: RefCell<HashSet<u64>>,
}

/// Bottom-right transient feedback, always delivered to the window that owns the workspace.
#[derive(Clone)]
pub(super) struct Toasts {
    window: AnyWindowHandle,
    state: Rc<State>,
}

impl Toasts {
    pub(super) fn new(window: &Window) -> Self {
        Self {
            window: window.window_handle(),
            state: Rc::default(),
        }
    }

    pub(super) fn show(&self, toast: Toast, cx: &mut App) {
        let serial = self.state.serial.get() + 1;
        self.state.serial.set(serial);
        let (window, state) = (self.window, self.state.clone());
        // Deferred so callers inside an entity or window update never re-enter it.
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                let lifetime = toast.lifetime;
                window.push_notification(toast.into_notification(serial, &state), cx);
                window
                    .spawn(cx, async move |cx| {
                        expire(serial, lifetime, state, cx).await
                    })
                    .detach();
            });
        });
    }
}

async fn expire(serial: u64, mut left: Duration, state: Rc<State>, cx: &mut AsyncWindowContext) {
    while !left.is_zero() {
        let step = left.min(TICK);
        cx.background_executor().timer(step).await;
        let Ok(visible) = cx.update(|window, _| window.is_visible()) else {
            return;
        };
        // Like Sonner, the countdown pauses while any toast is hovered or the window is hidden.
        if visible && state.hovered.borrow().is_empty() {
            left = left.saturating_sub(step);
        }
    }
    let _ = cx.update(|window, cx| window.remove_notification1::<ToastKey>(key(serial), cx));
}
