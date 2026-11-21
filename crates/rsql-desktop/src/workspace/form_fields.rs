use super::*;
use gpui_kit::{
    assets::IconName,
    base::{Checkbox, CheckboxState, ElementExt},
    component::{
        ActiveTheme, Icon, Sizable,
        input::{Input, InputContentType},
        select::{SearchableVec, Select, SelectItem},
        tooltip::Tooltip,
    },
    prelude::FluentBuilder,
};
use std::cell::Cell;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Field {
    Name,
    Host,
    Port,
    Database,
    Username,
    Password,
    SshHost,
    SshPort,
    SshUser,
    SshPassword,
    KeyPath,
}

impl Field {
    pub(super) const ALL: [Self; 11] = [
        Self::Name,
        Self::Host,
        Self::Port,
        Self::Database,
        Self::Username,
        Self::Password,
        Self::SshHost,
        Self::SshPort,
        Self::SshUser,
        Self::SshPassword,
        Self::KeyPath,
    ];

    pub(super) fn slot(self) -> usize {
        match self {
            Self::Name => 13,
            Self::Host => 4,
            Self::Port => 5,
            Self::Database => 3,
            Self::Username => 1,
            Self::Password => 2,
            Self::SshHost => 8,
            Self::SshPort => 9,
            Self::SshUser => 10,
            Self::SshPassword => 11,
            Self::KeyPath => 12,
        }
    }

    fn metadata(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Name => ("connection-name", "Connection Name", "production-db"),
            Self::Host => ("connection-host", "Host", "localhost"),
            Self::Port => ("connection-port", "Port", "5432"),
            Self::Database => ("connection-database", "Database", "mydb"),
            Self::Username => ("connection-username", "Username", "postgres"),
            Self::Password => ("connection-password", "Password", "••••••••"),
            Self::SshHost => ("connection-ssh-host", "SSH Host", "bastion.example.com"),
            Self::SshPort => ("connection-ssh-port", "SSH Port", "22"),
            Self::SshUser => ("connection-ssh-user", "SSH User", "ubuntu"),
            Self::SshPassword => ("connection-ssh-password", "SSH Password", "••••••••"),
            Self::KeyPath => ("connection-ssh-key", "Private Key Path", "~/.ssh/id_rsa"),
        }
    }

    pub(super) fn placeholder(self) -> &'static str {
        self.metadata().2
    }

    pub(super) fn masked(self) -> bool {
        matches!(self, Self::Password | Self::SshPassword)
    }

    fn ssh(self) -> bool {
        matches!(
            self,
            Self::SshHost | Self::SshPort | Self::SshUser | Self::SshPassword | Self::KeyPath
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SslMode {
    Disable,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

pub(super) type SslState = SelectState<SearchableVec<SslMode>>;

impl SslMode {
    pub(super) const ALL: [Self; 5] = [
        Self::Disable,
        Self::Prefer,
        Self::Require,
        Self::VerifyCa,
        Self::VerifyFull,
    ];

    pub(super) fn stored(value: &str) -> Self {
        let mode = rsql_desktop::connection::stored_mode(value);
        Self::ALL
            .into_iter()
            .find(|item| SslMode::value(*item) == mode)
            .unwrap_or(Self::VerifyFull)
    }

    pub(super) fn value(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyCa => "verify-ca",
            Self::VerifyFull => "verify-full",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Disable => "Unencrypted connection",
            Self::Prefer => "Prefer encryption without verifying the server",
            Self::Require => "Require encryption without verifying the server",
            Self::VerifyCa => "Verify certificate authority",
            Self::VerifyFull => "Verify certificate authority and host name",
        }
    }
}

impl SelectItem for SslMode {
    type Value = Self;

    fn title(&self) -> SharedString {
        match self {
            Self::Disable => "Disable — no TLS",
            Self::Prefer => "Prefer — TLS if available, unverified",
            Self::Require => "Require — TLS, unverified",
            Self::VerifyCa => "Verify CA — trusted certificate",
            Self::VerifyFull => "Verify full — trusted certificate and host name",
        }
        .into()
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

#[derive(Clone)]
pub(super) struct FocusRow {
    focus: FocusHandle,
    was_focused: Rc<Cell<bool>>,
    scroll: ScrollHandle,
}

impl FocusRow {
    pub(super) fn new(focus: FocusHandle, scroll: &ScrollHandle) -> Self {
        Self {
            focus,
            was_focused: Rc::new(Cell::new(false)),
            scroll: scroll.clone(),
        }
    }

    fn capture(&self, row: Div) -> Div {
        let row_focus = self.clone();
        row.on_prepaint(move |bounds, window, _| {
            let focused = row_focus.focus.is_focused(window);
            if row_focus.was_focused.replace(focused) || !focused {
                return;
            }
            let viewport = row_focus.scroll.bounds();
            let adjust = if bounds.top() < viewport.top() {
                viewport.top() - bounds.top()
            } else if bounds.bottom() > viewport.bottom() {
                viewport.bottom() - bounds.bottom()
            } else {
                return;
            };
            let offset = row_focus.scroll.offset();
            window.on_next_frame(move |window, _| {
                if row_focus.focus.is_focused(window) && row_focus.scroll.offset() == offset {
                    row_focus.scroll.set_offset(offset + point(px(0.), adjust));
                    window.refresh();
                }
            });
        })
    }
}

pub(super) fn safe_url_error(input: &str, error: String) -> String {
    let Ok(url) = url::Url::parse(input.trim()) else {
        return error;
    };
    let Some(password) = url.password().filter(|password| !password.is_empty()) else {
        return error;
    };
    let decoded = percent_encoding::percent_decode_str(password).decode_utf8_lossy();
    error
        .replace(password, "••••")
        .replace(decoded.as_ref(), "••••")
}

fn labeled(label: &'static str, ssh: bool, cx: &App) -> Div {
    div()
        .min_w_0()
        .flex()
        .flex_col()
        .flex_none()
        .gap(if ssh { px(4.) } else { px(8.) })
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_size(if ssh { px(11.) } else { px(12.) })
                .line_height(if ssh { px(16.5) } else { px(16.) })
                .text_color(if ssh {
                    cx.theme().muted_foreground
                } else {
                    cx.theme().foreground
                })
                .child(label),
        )
}

fn frame(cx: &App) -> Div {
    div()
        .w_full()
        .h(px(36.))
        .rounded(px(8.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.5))
        .bg(cx.theme().input.opacity(0.8))
        .text_size(px(14.))
        .line_height(px(20.))
}

struct TextField<'a> {
    id: &'static str,
    label: &'static str,
    state: &'a Entity<InputState>,
    row: &'a FocusRow,
    ssh: bool,
    masked: bool,
    error: bool,
    disabled: bool,
}

impl TextField<'_> {
    fn render(self, window: &Window, cx: &App) -> Div {
        let focus = self.state.focus_handle(cx);
        let focused = focus.is_focused(window) && !self.disabled;
        let height = if self.ssh { px(32.) } else { px(36.) };
        let control = frame(cx)
            .id(self.id)
            .h(height)
            .when(self.error, |field| field.border_color(cx.theme().danger))
            .when(self.disabled, |field| field.opacity(0.5))
            .shadow(vec![
                BoxShadow {
                    color: black().opacity(0.05),
                    offset: point(px(0.), px(1.)),
                    blur_radius: px(2.),
                    spread_radius: px(0.),
                    inset: false,
                },
                BoxShadow {
                    color: cx.theme().primary.opacity(if focused { 0.4 } else { 0. }),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                    inset: false,
                },
            ])
            .child(
                Input::new(self.state)
                    .aria_label(self.label)
                    .accessibility_id(self.id)
                    .appearance(false)
                    .disabled(self.disabled)
                    .when(self.masked, |input| {
                        input.content_type(InputContentType::Password)
                    })
                    .with_size(height)
                    .h_full()
                    .px(px(11.))
                    .py_0()
                    .text_size(px(14.))
                    .line_height(px(20.)),
            );
        self.row
            .capture(labeled(self.label, self.ssh, cx).child(control))
    }
}

impl Form {
    pub(super) fn field(&self, field: Field, window: &Window, cx: &App) -> Div {
        let (id, label, _) = field.metadata();
        TextField {
            id,
            label,
            row: &self.field_rows[field as usize],
            state: &self.fields[field as usize],
            ssh: field.ssh(),
            masked: field.masked(),
            error: false,
            disabled: self.saving,
        }
        .render(window, cx)
    }

    pub(super) fn url_fields(&self, window: &Window, cx: &App) -> Div {
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(8.))
            .child(
                TextField {
                    id: "connection-url",
                    label: "Connection URL",
                    row: &self.url_row,
                    state: &self.url,
                    ssh: false,
                    masked: true,
                    error: self.url_error.is_some(),
                    disabled: self.saving,
                }
                .render(window, cx),
            )
            .children(self.url_error.as_ref().map(|error| {
                div()
                    .id("connection-url-error")
                    .role(Role::Alert)
                    .text_color(cx.theme().danger)
                    .text_size(px(11.))
                    .line_height(px(16.5))
                    .child(super::feedback::preview(error))
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(div().h(px(1.)).flex_1().bg(cx.theme().border.opacity(0.3)))
                    .child(
                        div()
                            .px(px(8.))
                            .text_size(px(10.))
                            .line_height(px(15.))
                            .text_color(cx.theme().muted_foreground)
                            .child("or fill in manually"),
                    )
                    .child(div().h(px(1.)).flex_1().bg(cx.theme().border.opacity(0.3))),
            )
    }

    pub(super) fn ssl_field(&self, cx: &App) -> Div {
        let description = SslMode::stored(&self.profile.ssl).description();
        self.ssl_row.capture(
            div().flex_none().child(
                labeled("SSL mode", false, cx)
                    .child(
                        div().h(px(36.)).child(
                            Select::new(&self.ssl)
                                .id("connection-ssl")
                                .accessibility_label("SSL mode")
                                .disabled(self.saving)
                                .w_full()
                                .h(px(36.))
                                .px(px(11.))
                                .rounded(px(8.))
                                .border_color(cx.theme().border.opacity(0.5))
                                .bg(cx.theme().input.opacity(0.8))
                                .text_color(cx.theme().foreground)
                                .text_size(px(14.))
                                .line_height(px(20.)),
                        ),
                    )
                    .id("connection-ssl-field")
                    .tooltip(move |w, cx| Tooltip::new(description).build(w, cx)),
            ),
        )
    }

    pub(super) fn ssh_field(&self, window: &Window, cx: &Context<Workspace>) -> Div {
        let checked = self.profile.ssh_enabled == "true";
        let theme = cx.theme();
        let indicator = div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(16.))
            .rounded(px(4.))
            .border_1()
            .border_color(if checked { theme.primary } else { theme.border })
            .bg(if checked { theme.primary } else { theme.input })
            .when(checked, |indicator| {
                indicator.child(
                    Icon::new(IconName::Check)
                        .size(px(12.))
                        .text_color(theme.primary_foreground),
                )
            });
        let owner = cx.weak_entity();
        self.ssh_row.capture(
            div().pt(px(8.)).child(
                Checkbox::new("connection-ssh")
                    .accessibility_label("SSH Tunnel")
                    .track_focus(&self.ssh_focus)
                    .checked(checked)
                    .disabled(self.saving)
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(4.))
                    .when(
                        self.ssh_focus.is_focused(window) && !self.saving,
                        |checkbox| checkbox.shadow(vec![outline(theme.ring, px(1.))]),
                    )
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(12.))
                    .line_height(px(12.))
                    .child(indicator)
                    .child("SSH Tunnel")
                    .on_change(move |next, _, _, cx| {
                        let _ = owner.update(cx, |view, cx| {
                            if let Some(f) = view.navigation.form.as_mut().filter(|f| !f.saving) {
                                f.profile.ssh_enabled =
                                    (next == CheckboxState::Checked).to_string();
                                view.invalidate_form(cx);
                            }
                        });
                    }),
            ),
        )
    }
}

pub(super) fn outline(color: Hsla, spread_radius: Pixels) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius,
        inset: false,
    }
}

pub(super) fn driver(cx: &App) -> Div {
    labeled("Database Type", false, cx).child(
        frame(cx)
            .h(px(38.))
            .px(px(12.))
            .flex()
            .items_center()
            .child("PostgreSQL"),
    )
}
