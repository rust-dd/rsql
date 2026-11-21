use super::*;
use gpui_kit::component::label::Label;
use rsql_core::{
    Error,
    services::extension_action::{ExtensionAction, ExtensionReview},
};
use tokio::sync::watch;

mod actions;
mod controls;

pub(super) struct Extension {
    name: String,
    version: String,
    comment: String,
    installation: Option<Installation>,
}

struct Installation {
    default_version: String,
    schema: String,
}

impl Extension {
    pub(super) fn installed(row: Vec<String>) -> Result<Self, Error> {
        let [name, version, default, comment, schema] = row
            .try_into()
            .map_err(|_| Error::QueryFailed("Incomplete installed extension".into()))?;
        Ok(Self {
            name,
            version,
            comment,
            installation: Some(Installation {
                default_version: default,
                schema,
            }),
        })
    }

    pub(super) fn available(row: Vec<String>) -> Result<Self, Error> {
        let [name, version, comment] = row
            .try_into()
            .map_err(|_| Error::QueryFailed("Incomplete available extension".into()))?;
        Ok(Self {
            name,
            version,
            comment,
            installation: None,
        })
    }

    fn update_version(&self) -> Option<&str> {
        self.installation
            .as_ref()
            .map(|i| i.default_version.as_str())
            .filter(|v| !v.is_empty() && *v != self.version)
    }

    fn matches(&self, lower: &str) -> bool {
        self.name.to_lowercase().contains(lower) || self.comment.to_lowercase().contains(lower)
    }
}

struct Review {
    id: Uuid,
    statement: ExtensionReview,
    action: ExtensionAction,
    name: String,
}

#[derive(Default)]
pub(super) struct Extensions {
    pub(super) installed: Vec<Extension>,
    pub(super) available: Vec<Extension>,
    pub(super) available_section: Section,
    show_available: bool,
    review: Option<Review>,
    pub(super) running: Option<(Uuid, watch::Sender<()>)>,
    receipt: Option<(String, bool)>,
}

impl Extensions {
    pub(super) fn dismiss_review(&mut self) {
        self.review = None;
    }

    pub(super) fn wait_for_delivery(&mut self) {
        self.receipt = Some((
            "Waiting for the previous extension action to finish…".into(),
            false,
        ));
    }

    pub(super) fn cancel_write(&mut self) {
        if let Some((_, cancel)) = &mut self.running {
            cancel.send_replace(());
        }
    }
}

impl Panel {
    pub(super) fn reviewing_extension(&self) -> bool {
        self.extensions.review.is_some()
    }

    fn extension_tab(&self, available: bool, cx: &Context<Self>) -> impl IntoElement {
        let active = self.extensions.show_available == available;
        let (id, title, count) = if available {
            (
                "extensions-available",
                "Available",
                self.extensions.available.len(),
            )
        } else {
            (
                "extensions-installed",
                "Installed",
                self.extensions.installed.len(),
            )
        };
        let label = format!("{title} ({count})");
        div()
            .border_b_2()
            .border_color(if active {
                cx.theme().primary
            } else {
                gpui_kit::transparent_black()
            })
            .child(
                compact_button(Button::new(id))
                    .px_3()
                    .ghost()
                    .accessibility_label(label.clone())
                    .toggled(active)
                    .child(Label::new(label).text_color(if active {
                        cx.theme().foreground
                    } else {
                        cx.theme().muted_foreground
                    }))
                    .on_click(cx.listener(move |p, _, _, cx| {
                        p.extensions.show_available = available;
                        cx.notify();
                    })),
            )
    }

    fn extension_card(&self, row: &Extension, cx: &Context<Self>) -> impl IntoElement {
        let installed = row.installation.is_some();
        let (icon, color, badge_color, background) = if installed {
            (
                IconName::Check,
                theme::controls::extension_check_color(),
                cx.theme().primary,
                cx.theme().primary.opacity(0.1),
            )
        } else {
            (
                IconName::Download,
                cx.theme().muted_foreground,
                cx.theme().muted_foreground,
                cx.theme().muted,
            )
        };
        let mut title = div()
            .flex()
            .items_center()
            .gap_2()
            .child(Icon::new(icon).size(px(14.)).text_color(color))
            .child(
                Label::new(row.name.clone())
                    .id("extension-name")
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(14.)),
            )
            .child(
                div().rounded_full().px_2().py(px(2.)).bg(background).child(
                    Label::new(row.version.clone())
                        .id("extension-version")
                        .text_size(px(10.))
                        .text_color(badge_color),
                ),
            );
        if row.update_version().is_some() {
            title = title.child(self.extension_button(row, ExtensionAction::Update, cx));
        }
        title = title.child(div().flex_1());
        if let Some(installation) = &row.installation {
            title = title
                .child(view::secondary(
                    "extension-schema",
                    installation.schema.clone(),
                    cx,
                ))
                .child(self.extension_button(row, ExtensionAction::Drop, cx));
        } else {
            title = title.child(self.extension_button(row, ExtensionAction::Install, cx));
        }
        div()
            .id(SharedString::from(format!("extension-card-{}", row.name)))
            .mb_2()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .p_3()
            .hover(|s| s.bg(cx.theme().muted.opacity(0.3)))
            .child(title)
            .when(!row.comment.is_empty(), |d| {
                d.child(
                    div().mt_1().child(
                        view::secondary("extension-comment", row.comment.clone(), cx)
                            .text_size(px(11.)),
                    ),
                )
            })
    }

    pub(super) fn extensions_view(&self, cx: &Context<Self>) -> Div {
        let (rows, lane, section) = if self.extensions.show_available {
            (
                &self.extensions.available,
                Lane::Available,
                &self.extensions.available_section,
            )
        } else {
            (&self.extensions.installed, Lane::Main, &self.main)
        };
        let mut body = div()
            .id("extension-cards")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_4();
        let mut visible = rows
            .iter()
            .filter(|r| r.matches(&self.filters.text))
            .peekable();
        if visible.peek().is_none() && section.ready() {
            let empty = if !self.filters.text.is_empty() {
                "No matching extensions"
            } else if self.extensions.show_available {
                "No available extensions"
            } else {
                "No extensions installed"
            };
            body = body.child(
                div()
                    .py_8()
                    .text_center()
                    .child(view::secondary("extension-empty", empty, cx).text_size(px(12.))),
            );
        }
        body = body.children(visible.map(|r| self.extension_card(r, cx)));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                self.header(IconName::Package, "Extensions", cx)
                    .child(self.refresh_button(cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .px_4()
                    .py_2()
                    .child(self.extension_tab(false, cx))
                    .child(self.extension_tab(true, cx))
                    .child(div().flex_1())
                    .child(self.search_input(192.)),
            )
            .when_some(
                self.extensions.receipt.as_ref(),
                |d, (message, attention)| {
                    d.child(
                        div()
                            .mx_4()
                            .mt_2()
                            .when(*attention, |d| {
                                d.px_3()
                                    .py_2()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(cx.theme().danger.opacity(0.5))
                                    .bg(cx.theme().danger.opacity(0.1))
                            })
                            .child(
                                Label::new(message.clone())
                                    .id("extension-receipt")
                                    .text_color(if *attention {
                                        cx.theme().danger
                                    } else {
                                        cx.theme().foreground
                                    }),
                            ),
                    )
                },
            )
            .children(self.status(lane, section, cx))
            .child(body)
    }
}
