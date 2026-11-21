use super::{
    execution::Action,
    theme::controls::{accent_gradient, compact_button},
    *,
};
use gpui_kit::component::{
    ActiveTheme, Icon,
    button::{Button as MenuButton, ButtonVariants},
    label::Label,
    menu::{DropdownMenu, PopupMenuItem},
    tooltip::Tooltip,
};
use gpui_kit::{assets::IconName, base::Button};
use rsql_desktop::{document::Tab, execution::TIMEOUTS_MS, sql::is_blank};

pub(super) const ANALYZE_HINT: &str =
    "EXPLAIN ANALYZE executes the statement, including any data changes.";

fn modifier() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl"
    }
}

fn shortcut(keys: &str, color: Hsla, cx: &App) -> Label {
    Label::new(format!("{}+{keys}", modifier()))
        .font_family(cx.theme().mono_font_family.clone())
        .ml(px(2.))
        .text_size(px(10.))
        .text_color(color.opacity(0.6))
}

fn divider(cx: &App) -> Div {
    div()
        .w(px(1.))
        .h(px(16.))
        .mx_1()
        .bg(cx.theme().border.opacity(0.4))
}

fn icon(name: IconName, size: f32) -> Icon {
    Icon::new(name).size(px(size)).flex_shrink_0()
}

#[derive(Clone, Copy)]
enum Appearance {
    Ghost,
    Outline,
    Run,
    Stop,
    Right,
}

fn action_button(
    label: &'static str,
    glyph: Option<IconName>,
    hint: Option<Label>,
    appearance: Appearance,
    disabled: bool,
    cx: &App,
) -> Button {
    let theme = cx.theme();
    let hover = match appearance {
        Appearance::Ghost if theme.is_dark() => white().opacity(0.06),
        Appearance::Ghost => black().opacity(0.04),
        Appearance::Run => transparent_white(),
        Appearance::Stop => theme.danger.opacity(0.1),
        _ => theme.accent,
    };
    let mut button = Button::new(label)
        .accessibility_label(label)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .whitespace_nowrap()
        .text_size(px(12.))
        .line_height(px(16.))
        .h(px(28.))
        .px_2()
        .gap_1p5()
        .rounded(px(8.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme.foreground)
        .on_mouse_down(MouseButton::Left, |_, w, _| w.prevent_default())
        .hover(move |s| s.bg(hover))
        .disabled(disabled);
    if disabled {
        button = button.opacity(if matches!(appearance, Appearance::Right) {
            0.4
        } else {
            0.5
        });
    }
    button = match appearance {
        Appearance::Outline => button.border_1().border_color(theme.border),
        Appearance::Run => button
            .bg(accent_gradient())
            .text_color(white())
            .px_2p5()
            .hover(|s| {
                s.shadow(vec![
                    BoxShadow::new(px(0.), px(0.), rgb(0x7850dc).opacity(0.3).into())
                        .blur_radius(px(20.)),
                ])
            }),
        Appearance::Stop => button
            .border_1()
            .border_color(theme.danger.opacity(0.5))
            .text_color(theme.danger)
            .px_2p5(),
        Appearance::Right => button
            .border_1()
            .border_color(theme.border)
            .rounded(px(4.))
            .h(px(26.))
            .font_weight(FontWeight::NORMAL),
        Appearance::Ghost => button,
    };
    button
        .children(glyph.map(|name| {
            icon(
                name,
                if matches!(appearance, Appearance::Stop) {
                    12.
                } else {
                    14.
                },
            )
        }))
        .child(label)
        .children(hint)
}

fn with_tooltip(button: Button, message: impl Into<SharedString>) -> Button {
    let message = message.into();
    button.tooltip(move |w, cx| Tooltip::new(message.clone()).build(w, cx))
}

fn timeout_label(ms: Option<u32>) -> String {
    match ms {
        None => "No limit".into(),
        Some(ms) if ms >= 60_000 => format!("{}m", ms / 60_000),
        Some(ms) => format!("{}s", ms / 1000),
    }
}

impl Workspace {
    fn query_timeout(&self, tab: &Tab, cx: &Context<Self>) -> impl IntoElement {
        let id = tab.id.clone();
        let view = cx.weak_entity();
        let timeout = tab
            .timeout
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| TIMEOUTS_MS.contains(&Some(*n)));
        compact_button(MenuButton::new("query-timeout"))
            .ghost()
            .border_1()
            .border_color(cx.theme().border.opacity(0.4))
            .text_color(cx.theme().muted_foreground)
            .rounded(px(4.))
            .px_1p5()
            .label(timeout_label(timeout))
            .child(icon(IconName::ChevronsUpDown, 12.))
            .tooltip("Query timeout for both panes")
            .dropdown_menu(move |mut menu, _, _| {
                for &ms in TIMEOUTS_MS {
                    let id = id.clone();
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(timeout_label(ms))
                            .checked(ms == timeout)
                            .on_click(move |_, _, cx| {
                                let _ = view.update(cx, |v, cx| {
                                    if let Some(t) = v
                                        .session
                                        .documents
                                        .borrow_mut()
                                        .tabs
                                        .iter_mut()
                                        .find(|t| t.id == id)
                                    {
                                        t.timeout = ms.map(u64::from);
                                    }
                                    v.schedule_save(cx);
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            })
    }

    pub(super) fn query_toolbar(
        &self,
        tab: &Tab,
        side: Side,
        has_project: bool,
        show_shortcuts: bool,
        cx: &Context<Self>,
    ) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
        let owner = (tab.id.clone(), side);
        let pane = tab.pane(side);
        let blank = is_blank(&pane.draft.text);
        let selected = !pane.draft.selection.is_empty();
        let run_label = if selected {
            "Run selection"
        } else {
            "Run statement"
        };
        let run_title = if selected {
            "Runs the selected SQL as written"
        } else {
            "Runs the statement at the cursor"
        };
        let busy = pane.is_busy();
        let disabled = blank || !has_project || busy;
        let theme = cx.theme();
        let main = side == Side::Main;
        let mut bar = div()
            .id(if main {
                "main-query-toolbar"
            } else {
                "right-query-toolbar"
            })
            .test_support()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(if main {
                theme.border.opacity(0.4)
            } else {
                theme.border
            });
        let mut actions = div().flex().items_center().gap_1().flex_shrink_0();
        if main {
            let save_owner = owner.clone();
            let format_owner = owner.clone();
            let tab_id = tab.id.clone();
            let leading = div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    action_button(
                        "Save",
                        Some(IconName::Save),
                        None,
                        Appearance::Ghost,
                        blank || !has_project,
                        cx,
                    )
                    .on_click(cx.listener(move |v, _, w, cx| v.save_query(&save_owner, w, cx))),
                )
                .child(
                    action_button(
                        "Format",
                        Some(IconName::TextAlignStart),
                        None,
                        Appearance::Ghost,
                        blank,
                        cx,
                    )
                    .on_click(
                        cx.listener(move |v, _, w, cx| v.format_query(format_owner.clone(), w, cx)),
                    ),
                )
                .child(
                    with_tooltip(
                        action_button(
                            "Split",
                            Some(IconName::Columns2),
                            None,
                            if tab.split {
                                Appearance::Outline
                            } else {
                                Appearance::Ghost
                            },
                            false,
                            cx,
                        ),
                        "Toggle split editor",
                    )
                    .on_click(cx.listener(move |v, _, w, cx| {
                        v.update_documents(w, cx, |d| {
                            if let Some(t) = d.tabs.iter_mut().find(|t| t.id == tab_id) {
                                t.toggle_split();
                            }
                        })
                    })),
                )
                .child(divider(cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .flex_shrink_0()
                        .gap_1()
                        .child(icon(IconName::Timer, 12.).text_color(theme.muted_foreground))
                        .child(self.query_timeout(tab, cx)),
                )
                .child(divider(cx));
            bar = bar.bg(theme.group_box.opacity(0.4)).child(
                div()
                    .id("main-query-controls")
                    .min_w_0()
                    .overflow_x_scroll()
                    .child(leading.flex_shrink_0()),
            );
            for (label, glyph, action, shortcut, title) in [
                (
                    "Explain",
                    IconName::GitBranch,
                    Action::Explain,
                    show_shortcuts.then(|| shortcut("Shift+Enter", theme.muted_foreground, cx)),
                    None,
                ),
                (
                    "Analyze",
                    IconName::Gauge,
                    Action::Analyze,
                    None,
                    Some(ANALYZE_HINT),
                ),
            ] {
                let target = owner.clone();
                let mut button = action_button(
                    label,
                    Some(glyph),
                    shortcut,
                    Appearance::Outline,
                    disabled,
                    cx,
                )
                .on_click(
                    cx.listener(move |v, _, w, cx| v.run_query(target.clone(), action, w, cx)),
                );
                if let Some(title) = title {
                    button = with_tooltip(button, title);
                }
                actions = actions.child(button);
            }
        } else {
            bar = bar.justify_between().overflow_x_scroll().px_3().child(
                Label::new("Right query")
                    .text_xs()
                    .line_height(px(16.))
                    .text_color(theme.muted_foreground),
            );
            actions = actions.gap_1p5();
        }
        let right_or = |appearance| if main { appearance } else { Appearance::Right };
        if busy {
            let stop = action_button(
                if main { "Stop" } else { "Stop right query" },
                main.then_some(IconName::Square),
                (main && show_shortcuts).then(|| shortcut(".", theme.danger, cx)),
                right_or(Appearance::Stop),
                false,
                cx,
            )
            .on_click(cx.listener(move |v, _, _, cx| v.stop_query(&owner, cx)));
            return bar.child(actions.child(stop));
        }
        let target = owner.clone();
        let alt = if cfg!(target_os = "macos") {
            "⌥"
        } else {
            "Alt"
        };
        let script_title = if main {
            format!(
                "Runs every statement in the editor ({}+{alt}+Enter)",
                modifier()
            )
        } else {
            "Runs every statement in the right editor".into()
        };
        let script = with_tooltip(
            action_button(
                "Run script",
                main.then_some(IconName::ListOrdered),
                None,
                right_or(Appearance::Outline),
                disabled,
                cx,
            ),
            script_title,
        )
        .on_click(
            cx.listener(move |v, _, w, cx| v.run_query(target.clone(), Action::Script, w, cx)),
        );
        let run = with_tooltip(
            action_button(
                run_label,
                main.then_some(IconName::Play),
                (main && show_shortcuts).then(|| shortcut("Enter", white(), cx)),
                right_or(Appearance::Run),
                disabled,
                cx,
            ),
            run_title,
        )
        .on_click(cx.listener(move |v, _, w, cx| v.run_query(owner.clone(), Action::Run, w, cx)));
        bar.child(actions.child(script).child(run))
    }
}
