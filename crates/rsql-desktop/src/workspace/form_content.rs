use super::{fields::outline, *};
use gpui_kit::{
    assets::IconName,
    base::Button as Press,
    component::{
        ActiveTheme, Icon, Sizable, dialog::DialogEntrance, spinner::Spinner, theme::oklch,
    },
    prelude::FluentBuilder,
};

pub(super) fn open(owner: &Entity<Workspace>, w: &mut Window, cx: &mut App) {
    let content = cx.new(|cx| FormContent {
        _observe: cx.observe(owner, |_, _, cx| cx.notify()),
        owner: owner.downgrade(),
    });
    let owner = owner.downgrade();
    w.open_dialog(cx, move |dialog, w, cx| {
        let (dismissible, initial_focus, name) = owner
            .upgrade()
            .and_then(|view| {
                let form = view.read(cx).navigation.form.as_ref()?;
                let focus = form.initial_focus.replace(false).then(|| {
                    match form.mode {
                        Mode::New => &form.url,
                        Mode::Edit | Mode::Reconnect => &form.fields[Field::Name as usize],
                    }
                    .focus_handle(cx)
                });
                Some((!form.saving, focus, Some(form_title(form))))
            })
            .unwrap_or_default();
        if let Some(focus) = initial_focus {
            focus.focus(w, cx);
        }
        let close = owner.clone();
        let cancel = owner.clone();
        let content = content.clone();
        let width = if w.viewport_size().width >= px(640.) {
            px(500.)
        } else {
            px(448.)
        };
        dialog
            .when_some(name, |dialog, name| dialog.accessibility_label(name))
            .w(width)
            .centered()
            .entrance(DialogEntrance::Fade)
            .p_0()
            .gap_0()
            .rounded(px(12.))
            .overflow_hidden()
            .bg(cx.theme().group_box)
            .border_color(cx.theme().border.opacity(0.5))
            .backdrop_color(black().opacity(0.6))
            .shadow(vec![
                outline(white().opacity(0.05), px(1.)),
                BoxShadow {
                    color: black().opacity(0.5),
                    offset: point(px(0.), px(25.)),
                    blur_radius: px(50.),
                    spread_radius: px(-12.),
                    inset: false,
                },
            ])
            .close_button(false)
            .overlay_closable(dismissible)
            .keyboard(dismissible)
            .on_ok(|_, _, _| false)
            .on_cancel(move |_, _, cx| {
                cancel.upgrade().is_none_or(|owner| {
                    !owner
                        .read(cx)
                        .navigation
                        .form
                        .as_ref()
                        .is_some_and(|form| form.saving)
                })
            })
            .on_close(move |_, _, cx| {
                let _ = close.update(cx, |view, cx| {
                    view.close_form(cx);
                    view.navigation.modal_open = false;
                    cx.notify();
                });
            })
            .content(move |body, _, _| {
                body.gap_0()
                    .min_h_0()
                    .child(clipboard_order::capture(div()).child(content.clone()))
            })
    });
}

fn form_title(form: &Form) -> &'static str {
    match form.challenge.as_ref().map(|(key, _, _)| key) {
        Some(key) if key.previous_fingerprint.is_some() => "SSH server key changed",
        Some(_) => "Verify SSH server",
        None => match form.mode {
            Mode::New => "New Connection",
            Mode::Edit => "Edit Connection",
            Mode::Reconnect => "Reconnect",
        },
    }
}

struct FormContent {
    owner: WeakEntity<Workspace>,
    _observe: Subscription,
}

impl Render for FormContent {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.owner
            .update(cx, |v, cx| {
                v.form_element(w, cx).map(IntoElement::into_any_element)
            })
            .ok()
            .flatten()
            .unwrap_or_else(|| div().into_any_element())
    }
}

enum ButtonStyle {
    Outline,
    Ghost,
    Primary,
    Close,
}

fn button_style(button: Press, variant: ButtonStyle, cx: &App) -> Press {
    let theme = cx.theme();
    let ghost = if theme.is_dark() {
        white().opacity(0.06)
    } else {
        black().opacity(0.04)
    };
    let ring = theme.ring;
    let button = button
        .h(px(36.))
        .px(px(12.))
        .gap(px(6.))
        .rounded(px(8.))
        .flex_shrink_0()
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(theme.foreground)
        .whitespace_nowrap()
        .styles(|styles| styles.disabled(|style| style.opacity(0.5)))
        .focus_visible(move |style| style.shadow(vec![outline(ring, px(2.))]));
    match variant {
        ButtonStyle::Outline => {
            let accent = theme.accent;
            let fg = theme.accent_foreground;
            button
                .border_1()
                .border_color(theme.border)
                .hover(move |s| s.bg(accent).text_color(fg))
        }
        ButtonStyle::Ghost => {
            let fg = theme.accent_foreground;
            button.hover(move |s| s.bg(ghost).text_color(fg))
        }
        ButtonStyle::Primary => button
            .bg(theme::controls::accent_gradient())
            .text_color(white())
            .hover(|s| {
                s.shadow(vec![BoxShadow {
                    color: rgb(0x7850dc).opacity(0.3).into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(20.),
                    spread_radius: px(0.),
                    inset: false,
                }])
            }),
        ButtonStyle::Close => {
            let hovered = theme.muted.opacity(0.5);
            let fg = theme.foreground;
            button
                .size(px(28.))
                .px_0()
                .text_color(theme.muted_foreground.opacity(0.6))
                .hover(move |s| s.bg(hovered).text_color(fg))
        }
    }
}

impl Workspace {
    fn press(
        &self,
        label: &'static str,
        variant: ButtonStyle,
        cx: &Context<Self>,
        callback: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Press {
        button_style(
            Press::new(label)
                .accessibility_label(label)
                .on_click(cx.listener(move |v, _, w, cx| callback(v, w, cx))),
            variant,
            cx,
        )
    }

    fn form_element(&self, w: &Window, cx: &Context<Self>) -> Option<impl IntoElement> {
        let f = self.navigation.form.as_ref()?;
        let title = form_title(f);
        let description = match f.mode {
            Mode::New => "Add a new database connection",
            Mode::Edit => "Update connection details",
            Mode::Reconnect => "Update and reconnect to the saved database",
        };
        let challenge = f.challenge.as_ref().map(|(key, _, _)| key);
        let available = w.fully_visible_bounds().size;
        let margin = cx.theme().spacing_tokens().lg + w.client_inset().unwrap_or_default();
        let available_height = available.height - margin * 2. - px(2.);
        let available_width = available.width - margin * 2.;
        let padding = if available_width < px(380.) {
            px(16.)
        } else {
            px(24.)
        };
        let body = div()
            .id("connection-form")
            .flex()
            .flex_col()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&f.scroll)
            .gap(px(16.))
            .px(padding);
        let (body, footer) = if let Some(key) = challenge {
            (
                body.child(challenge_details(key)),
                self.trust_footer(key, cx),
            )
        } else {
            (
                body.when(matches!(f.mode, Mode::New), |body| {
                    body.child(f.url_fields(w, cx))
                })
                .child(fields::driver(cx))
                .child(f.field(Field::Name, w, cx))
                .child(field_pair(f, Field::Host, Field::Port, px(16.), w, cx))
                .child(f.field(Field::Database, w, cx))
                .child(f.field(Field::Username, w, cx))
                .child(f.field(Field::Password, w, cx))
                .child(f.ssl_field(cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_none()
                        .gap(px(8.))
                        .child(f.ssh_field(w, cx))
                        .when(f.profile.ssh_enabled == "true", |section| {
                            section.child(ssh_panel(f, w, cx))
                        }),
                )
                .children(
                    f.feedback
                        .as_ref()
                        .map(|feedback| self.feedback(feedback, cx)),
                ),
                self.form_footer(f, cx),
            )
        };
        Some(
            div()
                .id("connection-dialog")
                .w_full()
                .max_h(available_height.max(px(0.)))
                .flex()
                .flex_col()
                .relative()
                .pt(px(24.))
                .pb(px(24.))
                .text_color(cx.theme().foreground)
                .child(
                    div()
                        .id("connection-dialog-header")
                        .px(padding)
                        .pr(padding + px(24.))
                        .pb(px(16.))
                        .flex_none()
                        .child(
                            div()
                                .id("connection-dialog-title")
                                .role(Role::Heading)
                                .aria_label(title)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(px(18.))
                                .line_height(px(28.))
                                .child(title),
                        )
                        .when(challenge.is_none(), |header| {
                            header.child(
                                div()
                                    .id("connection-dialog-description")
                                    .aria_label(description)
                                    .mt(px(4.))
                                    .text_size(px(12.))
                                    .line_height(px(16.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(description),
                            )
                        }),
                )
                .child(body)
                .child(footer.px(padding))
                .child(
                    self.press("Close", ButtonStyle::Close, cx, |v, _, cx| v.close_form(cx))
                        .disabled(f.saving)
                        .absolute()
                        .top(px(12.))
                        .right(px(12.))
                        .child(Icon::new(IconName::X).size(px(16.))),
                ),
        )
    }

    fn form_footer(&self, f: &Form, cx: &Context<Self>) -> Div {
        let pending = f.job.as_ref().map(|job| job.op);
        let disabled = self.navigation.busy || f.saving || pending.is_some();
        let draft = self.draft(cx).unwrap();
        let primary = match (f.mode, f.saving, pending) {
            (_, true, _) => "Saving…",
            (_, _, Some(Operation::Connect)) => "Connecting…",
            (Mode::New, _, _) => "Connect",
            (Mode::Edit, _, _) => "Save Changes",
            (Mode::Reconnect, _, _) => "Reconnect",
        };
        footer()
            .child(
                self.press("Test Connection", ButtonStyle::Outline, cx, |v, _, cx| {
                    v.submit(Operation::Test, cx);
                })
                .accessibility_label(if pending == Some(Operation::Test) {
                    "Testing connection…"
                } else {
                    "Test Connection"
                })
                .disabled(disabled || draft.check_route().is_err())
                .when(pending == Some(Operation::Test), |button| {
                    button.child(
                        Spinner::new()
                            .with_size(px(12.))
                            .color(cx.theme().foreground),
                    )
                })
                .child("Test Connection"),
            )
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .gap(px(8.))
                    .child(
                        self.press("Cancel", ButtonStyle::Ghost, cx, |v, _, cx| {
                            v.close_form(cx)
                        })
                        .disabled(f.saving)
                        .child("Cancel"),
                    )
                    .child(
                        self.press(primary, ButtonStyle::Primary, cx, |v, _, cx| {
                            v.submit_primary(cx)
                        })
                        .disabled(disabled || draft.check_form().is_err())
                        .child(primary),
                    ),
            )
    }

    fn trust_footer(&self, key: &Challenge, cx: &Context<Self>) -> Div {
        let id = key.id.clone();
        let serial = self.navigation.serial;
        let label = if key.previous_fingerprint.is_some() {
            "Replace trusted key"
        } else {
            "Trust this key"
        };
        footer()
            .justify_end()
            .child(
                self.press("Cancel connection", ButtonStyle::Ghost, cx, |v, _, cx| {
                    v.close_form(cx)
                })
                .child("Cancel connection"),
            )
            .child(
                self.press(label, ButtonStyle::Primary, cx, move |v, _, cx| {
                    v.approve(serial, &id, cx)
                })
                .child(label),
            )
    }

    fn feedback(&self, feedback: &Feedback, cx: &Context<Self>) -> impl IntoElement {
        let (id, message, glyph, color) = match feedback {
            Feedback::Success(text) => (
                "connection-success",
                text,
                IconName::CircleCheck,
                oklch(0.696, 0.17, 162.48),
            ),
            Feedback::Error(text) => (
                "connection-error",
                text,
                IconName::CircleX,
                cx.theme().danger,
            ),
        };
        let preview = message.preview.clone();
        let long = message.can_copy();
        let original = message.full.clone();
        div()
            .id(id)
            .flex_none()
            .role(Role::Alert)
            .aria_label(preview.clone())
            .flex()
            .items_start()
            .gap(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(color.opacity(0.3))
            .bg(color.opacity(0.05))
            .px(px(12.))
            .py(px(8.))
            .text_color(color)
            .text_size(px(12.))
            .line_height(px(16.))
            .child(Icon::new(glyph).size(px(14.)).mt(px(2.)).flex_shrink_0())
            .child(
                div()
                    .min_w_0()
                    .whitespace_normal()
                    .child(preview)
                    .when(long, |message| {
                        message.child(
                            self.press(
                                "Copy full result",
                                ButtonStyle::Outline,
                                cx,
                                move |v, _, cx| v.copy_form_result(original.clone(), cx),
                            )
                            .mt(px(8.))
                            .child("Copy full result"),
                        )
                    }),
            )
    }
}

fn footer() -> Div {
    div()
        .flex()
        .flex_wrap()
        .flex_none()
        .justify_between()
        .items_start()
        .gap(px(8.))
        .pt(px(32.))
}

fn field_pair(f: &Form, first: Field, second: Field, gap: Pixels, w: &Window, cx: &App) -> Div {
    div()
        .flex()
        .flex_none()
        .gap(gap)
        .child(div().min_w_0().flex_1().child(f.field(first, w, cx)))
        .child(div().min_w_0().flex_1().child(f.field(second, w, cx)))
}

fn ssh_panel(f: &Form, w: &Window, cx: &App) -> impl IntoElement {
    div()
        .id("connection-ssh-panel")
        .flex()
        .flex_col()
        .gap(px(12.))
        .p(px(12.))
        .rounded(px(8.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.5))
        .bg(cx.theme().muted.opacity(0.2))
        .child(field_pair(
            f,
            Field::SshHost,
            Field::SshPort,
            px(12.),
            w,
            cx,
        ))
        .child(f.field(Field::SshUser, w, cx))
        .child(f.field(Field::SshPassword, w, cx))
        .child(f.field(Field::KeyPath, w, cx))
}

fn challenge_details(key: &Challenge) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .text_size(px(12.))
        .line_height(px(16.))
        .child("Verify the fingerprint with your server administrator before trusting this key.")
        .child(format!("Server {}:{}", key.host, key.port))
        .children(
            key.previous_fingerprint
                .as_ref()
                .map(|prev| format!("Previously trusted: {prev}")),
        )
        .child(format!("New fingerprint: {}", key.fingerprint))
}
