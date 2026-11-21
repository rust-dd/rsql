use crate::TestSupportExt as _;
use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, FocusHandle, InteractiveElement as _, IntoElement, KeyBinding,
    MouseButton, ParentElement, Pixels, RenderOnce, StyleRefinement, Styled, Window, anchored, div,
    point, prelude::FluentBuilder as _, px,
};

use crate::{FocusTrapElement as _, StyledExt as _, actions::Cancel};

const CONTEXT: &str = "Sheet";

type CloseHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type CloseRequest = Rc<dyn Fn(&mut Window, &mut App)>;

fn close(request: &CloseRequest, notify: &CloseHandler, window: &mut Window, cx: &mut App) {
    let event = ClickEvent::default();
    request(window, cx);
    notify(&event, window, cx);
}

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Cancel, Some(CONTEXT))]);
}

/// An unstyled modal sheet host.
///
/// Applications provide the overlay and surface. The host owns focus trapping,
/// Escape handling, overlay dismissal, and close callback ordering.
#[derive(IntoElement)]
pub struct Sheet {
    base: gpui::Div,
    style: StyleRefinement,
    focus: FocusHandle,
    overlay_interactive: bool,
    overlay_closable: bool,
    dismiss_before_y: Option<Pixels>,
    overlay: Option<AnyElement>,
    surface: Option<AnyElement>,
    request_close: CloseRequest,
    on_close: CloseHandler,
}

impl Sheet {
    pub fn new(cx: &mut App) -> Self {
        Self {
            base: div(),
            style: StyleRefinement::default(),
            focus: cx.focus_handle(),
            overlay_interactive: true,
            overlay_closable: true,
            dismiss_before_y: None,
            overlay: None,
            surface: None,
            request_close: Rc::new(|_, _| {}),
            on_close: Rc::new(|_, _, _| {}),
        }
    }

    pub fn overlay(mut self, overlay: impl IntoElement) -> Self {
        self.overlay = Some(overlay.into_any_element());
        self
    }

    pub fn surface(mut self, surface: impl IntoElement) -> Self {
        self.surface = Some(surface.into_any_element());
        self
    }

    pub fn overlay_closable(mut self, closable: bool) -> Self {
        self.overlay_closable = closable;
        self
    }

    #[doc(hidden)]
    pub fn overlay_interactive(mut self, interactive: bool) -> Self {
        self.overlay_interactive = interactive;
        self
    }

    pub fn on_close(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Rc::new(handler);
        self
    }

    #[doc(hidden)]
    pub fn focus_handle(mut self, focus: FocusHandle) -> Self {
        self.focus = focus;
        self
    }

    #[doc(hidden)]
    pub fn dismiss_before_y(mut self, y: Pixels) -> Self {
        self.dismiss_before_y = Some(y);
        self
    }

    #[doc(hidden)]
    pub fn request_close(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.request_close = Rc::new(handler);
        self
    }
}

impl Styled for Sheet {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Sheet {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let viewport = window.viewport_size();
        let request_close = self.request_close;
        let on_close = self.on_close;
        let escape_request = request_close.clone();
        let escape_notify = on_close.clone();

        anchored().position(point(px(0.), px(0.))).child(
            self.base
                .id("sheet-host")
                .test_support()
                .absolute()
                .top_0()
                .left_0()
                .w(viewport.width)
                .h(viewport.height)
                .key_context(CONTEXT)
                .track_focus(&self.focus)
                .focus_trap("sheet", &self.focus)
                .on_action(move |_: &Cancel, window, cx| {
                    cx.propagate();
                    close(&escape_request, &escape_notify, window, cx);
                })
                .when_some(self.overlay, |this, overlay| {
                    let request_close = request_close.clone();
                    let on_close = on_close.clone();
                    let dismiss_before_y = self.dismiss_before_y;
                    let overlay_interactive = self.overlay_interactive;
                    let overlay_closable = self.overlay_closable;
                    this.child(overlay).child(div().absolute().inset_0().when(
                        overlay_interactive,
                        |this| {
                            this.on_any_mouse_down(move |event, window, cx| {
                                if !overlay_interactive {
                                    return;
                                }
                                if dismiss_before_y.is_some_and(|top| event.position.y < top) {
                                    return;
                                }
                                cx.stop_propagation();
                                if overlay_closable && event.button == MouseButton::Left {
                                    close(&request_close, &on_close, window, cx);
                                }
                            })
                        },
                    ))
                })
                .children(self.surface)
                .refine_style(&self.style),
        )
    }
}
