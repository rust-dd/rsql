use super::export_jobs::ExportRequest;
use super::*;
use crate::workspace::theme::controls::compact_button;
use gpui_kit::component::Disableable;
use gpui_kit::{assets::IconName, base::Button, component::Icon};
use rsql_desktop::results::export::Format;

const ITEMS: [(&str, Format, bool); 9] = [
    ("Save CSV", Format::Csv, false),
    ("Save JSON", Format::Json, false),
    ("Save SQL", Format::Sql, false),
    ("Save Markdown", Format::Markdown, false),
    ("Save XML", Format::Xml, false),
    ("Copy CSV", Format::Csv, true),
    ("Copy JSON", Format::Json, true),
    ("Copy SQL", Format::Sql, true),
    ("Copy Markdown", Format::Markdown, true),
];

pub(in crate::workspace) struct ExportMenu {
    owner: Owner,
    id: Uuid,
    items: Vec<FocusHandle>,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = rsql, no_json)]
struct ExportTab {
    reverse: bool,
}
pub(super) fn bind_keys(cx: &mut App) {
    for (key, reverse) in [("tab", false), ("shift-tab", true)] {
        cx.bind_keys([KeyBinding::new(
            key,
            ExportTab { reverse },
            Some("RsqlExportMenu"),
        )]);
    }
}

impl Workspace {
    pub(in crate::workspace) fn sync_export_menu(
        &mut self,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.export_menu.as_ref() else {
            return;
        };
        let docs = self.session.documents.borrow();
        let local = docs
            .active()
            .filter(|tab| tab.id == menu.owner.0)
            .and_then(|tab| tab.main.view())
            .is_some_and(|v| {
                v.data.id == menu.id
                    && v.export_job.is_none()
                    && v.snapshot(ResultAction::Export).is_some()
            });
        if !local {
            if menu.items.iter().any(|item| item.is_focused(w))
                && let Some(editor) = self.editors.get(&menu.owner)
            {
                editor.export_focus.focus(w, cx);
            }
            self.export_menu = None;
        }
    }
    fn leave_export_menu(&mut self, reverse: bool, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.export_menu.take() {
            let Some(editor) = self.editors.get(&menu.owner) else {
                cx.notify();
                return;
            };
            let destination = if reverse {
                editor
                    .search
                    .as_ref()
                    .map(|search| search.state.focus_handle(cx))
            } else {
                let docs = self.session.documents.borrow();
                docs.active()
                    .filter(|tab| tab.id == menu.owner.0)
                    .and_then(|tab| self.rendered_result_grid(tab, menu.owner.1, cx))
                    .filter(|grid| grid.read(cx).id() == menu.id)
                    .map(|grid| grid.read(cx).table.focus_handle(cx))
            };
            destination
                .unwrap_or_else(|| editor.state.focus_handle(cx))
                .focus(w, cx);
            cx.notify();
        }
    }

    pub(in crate::workspace) fn export_toolbar(
        &self,
        owner: Owner,
        view: &ResultView,
        cx: &Context<Self>,
    ) -> Div {
        let bar = div().flex().flex_wrap().items_center().gap_2().text_xs();
        let id = view.data.id;
        if let Some(job) = &view.export_job {
            let label = if job.retained {
                format!(
                    "Exporting {} / {} retained rows",
                    *job.progress.borrow(),
                    view.data.total_rows
                )
            } else {
                "Exporting…".into()
            };
            let disabled = *job.cancel.borrow();
            let button = self.button("Stop export", cx, move |v, _, cx| {
                let docs = v.session.documents.borrow();
                if let Some(job) = docs
                    .active()
                    .filter(|t| t.id == owner.0)
                    .and_then(|t| t.pane(owner.1).view())
                    .filter(|r| r.data.id == id)
                    .and_then(|r| r.export_job.as_ref())
                {
                    job.cancel.send_replace(true);
                    cx.notify();
                }
            });
            return bar
                .child(label)
                .child(compact_button(button).disabled(disabled));
        }
        if !view.allows(ResultAction::Export) {
            return bar;
        }
        let Some(live) = self.editors.get(&owner) else {
            return bar;
        };
        let packed = view.snapshot(ResultAction::Export).is_none();
        let menu = self
            .export_menu
            .as_ref()
            .filter(|m| !packed && m.owner == owner && m.id == id);
        let menu_open = menu.is_some();
        let action = owner.clone();
        let label = if packed { "Export CSV" } else { "Export" };
        let mut trigger = super::toolbar::result_action(
            self.button(label, cx, move |v, w, cx| {
                if packed {
                    v.start_export(&action, id, ExportRequest::RetainedCsv, cx);
                } else if menu_open {
                    v.export_menu = None;
                } else {
                    let items: Vec<_> = ITEMS.iter().map(|_| cx.focus_handle()).collect();
                    items[0].focus(w, cx);
                    v.export_menu = Some(ExportMenu {
                        owner: action.clone(),
                        id,
                        items,
                    });
                }
                cx.notify();
            }),
            gpui_kit::assets::IconName::Download,
            cx,
        )
        .track_focus(&live.export_focus);
        if packed {
            let cap = if view.data.capped {
                " (truncated result)"
            } else {
                ""
            };
            trigger = trigger.tooltip(format!(
                "Export {} retained rows{cap}. Choose a new file. Stopping leaves a partial CSV.",
                view.data.total_rows
            ));
        }
        let mut control = div().relative().child(trigger);
        if let Some(menu) = menu {
            let mut list = div()
                .id("result-export-menu")
                .test_support()
                .role(Role::Menu)
                .occlude()
                .flex()
                .flex_col()
                .py_1()
                .w(px(208.))
                .rounded(px(6.))
                .shadow(vec![
                    BoxShadow::new(px(0.), px(4.), black().opacity(0.1))
                        .blur_radius(px(6.))
                        .spread_radius(px(-1.)),
                    BoxShadow::new(px(0.), px(2.), black().opacity(0.1))
                        .blur_radius(px(4.))
                        .spread_radius(px(-2.)),
                ])
                .bg(cx.theme().popover)
                .border_1()
                .border_color(cx.theme().border)
                .key_context("RsqlExportMenu")
                .on_action(cx.listener(|v, a: &ExportTab, w, cx| {
                    cx.stop_propagation();
                    v.leave_export_menu(a.reverse, w, cx);
                }))
                .capture_key_down(cx.listener(|v, event: &KeyDownEvent, w, cx| {
                    let Some(m) = &v.export_menu else {
                        return;
                    };
                    let i = m.items.iter().position(|f| f.is_focused(w)).unwrap_or(0);
                    match event.keystroke.key.as_str() {
                        "up" => m.items[(i + ITEMS.len() - 1) % ITEMS.len()].focus(w, cx),
                        "down" => m.items[(i + 1) % ITEMS.len()].focus(w, cx),
                        "escape" => {
                            v.editors[&m.owner].export_focus.focus(w, cx);
                            v.export_menu = None;
                        }
                        "enter" | "space" => {
                            let (_, format, copy) = ITEMS[i];
                            let (owner, id) = (m.owner.clone(), m.id);
                            v.editors[&owner].export_focus.focus(w, cx);
                            v.export_menu = None;
                            v.start_export(&owner, id, ExportRequest::Local { format, copy }, cx);
                        }
                        _ => return,
                    }
                    cx.stop_propagation();
                    cx.notify();
                }))
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|v, _, _, cx| {
                        v.export_menu = None;
                        cx.notify();
                    }),
                );
            for (i, (label, format, copy)) in ITEMS.into_iter().enumerate() {
                if i == 0 || (copy && !ITEMS[i - 1].2) {
                    if i != 0 {
                        list = list.child(div().h(px(1.)).my_1().bg(cx.theme().border));
                    }
                    list = list.child(
                        div()
                            .px_2()
                            .py_1()
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().muted_foreground)
                            .child(theme::controls::tracked(
                                if copy {
                                    "COPY TO CLIPBOARD"
                                } else {
                                    "DOWNLOAD"
                                },
                                10.,
                                0.05,
                            )),
                    );
                }
                let action = owner.clone();
                let text = label.split_once(' ').unwrap().1.to_uppercase();
                list = list.child(
                    Button::new(label)
                        .accessibility_label(label)
                        .role(Role::MenuItem)
                        .track_focus(&menu.items[i])
                        .flex()
                        .items_center()
                        .w_full()
                        .h(px(28.))
                        .px_3()
                        .gap_2()
                        .text_xs()
                        .line_height(px(16.))
                        .hover(|s| s.bg(cx.theme().accent))
                        .child(
                            Icon::new(if copy {
                                IconName::Copy
                            } else {
                                IconName::Download
                            })
                            .size(px(12.))
                            .text_color(cx.theme().muted_foreground),
                        )
                        .child(text)
                        .on_click(cx.listener(move |v, _: &ClickEvent, w, cx| {
                            v.editors[&action].export_focus.focus(w, cx);
                            v.export_menu = None;
                            v.start_export(&action, id, ExportRequest::Local { format, copy }, cx);
                        })),
                );
            }
            control = control.child(
                div().absolute().right_0().top(px(24.)).size_0().child(
                    deferred(
                        anchored()
                            .anchor(Anchor::TopRight)
                            .snap_to_window_with_margin(px(8.))
                            .child(list),
                    )
                    .with_priority(2),
                ),
            );
        }
        bar.child(control).children(view.export_message.clone())
    }
}
