use super::*;
use gpui_kit::{
    assets::IconName,
    component::{ActiveTheme, input::Input},
    prelude::FluentBuilder,
};

#[derive(PartialEq, Eq)]
pub(super) struct RenderedChoice {
    source: EntityId,
    page: Page,
    search: SharedString,
    command: Command,
}

impl RenderedChoice {
    fn current(&self, p: &Palette, cx: &App) -> bool {
        !p.workspace_operation.writing()
            && p.search.entity_id() == self.source
            && p.page == self.page
            && p.search.read(cx).value() == self.search
    }
}

impl Workspace {
    fn click_palette_choice(
        &mut self,
        choice: &RenderedChoice,
        event: &ClickEvent,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = &mut self.palette else { return };
        let pressed = p.pressed.take();
        if !choice.current(p, cx)
            || (matches!(event, ClickEvent::Mouse(_)) && pressed.as_deref() != Some(choice))
        {
            return;
        }
        let entries = self.entries(self.palette.as_ref().unwrap(), cx);
        if let Some(index) = entries.iter().position(|(_, c)| c == &choice.command) {
            self.palette.as_mut().unwrap().choose_index(index, &entries);
            self.run_command(choice.command.clone(), w, cx);
            cx.notify();
        }
    }

    pub(super) fn palette_back(&self, cx: &Context<Self>) -> gpui_kit::base::Button {
        gpui_kit::base::Button::new("Back")
            .accessibility_label("Back")
            .size(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .hover(|button| button.bg(cx.theme().accent))
            .child(workspaces::workspace_icon(IconName::ArrowLeft, false, cx))
            .on_click(cx.listener(|v, _: &ClickEvent, w, cx| v.page(Page::Root, w, cx)))
    }

    pub(in crate::workspace) fn palette_view(&mut self, cx: &Context<Self>) -> Option<Div> {
        self.palette.as_mut()?.sync_search_selection(cx);
        let p = self.palette.as_ref().unwrap();
        let entries = self.entries(p, cx);
        let p = self.palette.as_mut().unwrap();
        p.update_row_layout(&entries);
        let selected = p.reconcile_selection(&entries);
        let p = self.palette.as_ref().unwrap();
        let view = cx.weak_entity();
        let page = p.page;
        let layout = p.row_layout.clone();
        let writing = p.workspace_operation.writing();
        let status = self.navigation.status.clone();
        let session = self.session.clone();
        let source = p.search.entity_id();
        let search = p.selected_search.clone();
        let rows = if entries.is_empty() {
            if matches!(page, Page::SaveWorkspace | Page::SaveQuery)
                || (page.workspace_list() && self.session.named_workspaces().loading)
            {
                div()
            } else {
                div()
                    .px_4()
                    .py_8()
                    .text_center()
                    .text_color(cx.theme().muted_foreground)
                    .child(if page.workspace_list() {
                        "No saved workspaces"
                    } else {
                        "No results found"
                    })
            }
        } else {
            let height = layout
                .iter()
                .map(|row| row.height(page))
                .sum::<f32>()
                .min(392.);
            div().py_1().child(
                list(p.scroll.clone(), move |row_index, _, cx| {
                    let row = &layout[row_index];
                    let height = row.height(page);
                    let i = match row {
                        presentation::Row::Heading(group) => {
                            let label = group.label();
                            return div()
                                .id(group.id())
                                .test_support()
                                .role(Role::Heading)
                                .aria_label(label.clone())
                                .h(px(height))
                                .px_4()
                                .flex()
                                .items_center()
                                .text_size(px(10.))
                                .line_height(px(15.))
                                .text_color(cx.theme().muted_foreground.opacity(0.7))
                                .child(theme::controls::tracked(label.to_uppercase(), 10., 0.05))
                                .into_any_element();
                        }
                        presentation::Row::Choice(index) => *index,
                    };
                    let library = session.named_workspaces();
                    let (label, command) = entries[i].clone();
                    let view = view.clone();
                    let profile = match &command {
                        Command::Connection(p) => Some((
                            format!("{}:{}/{}", p.host, p.port, p.database),
                            status
                                .get(&p.id)
                                .is_some_and(|status| status == "Connected"),
                        )),
                        _ => None,
                    };
                    let (accessibility, content) =
                        workspaces::row_content(label, &command, &library, profile, cx);
                    let description = match &command {
                        Command::Draft(_, draft) => draft.detail.clone(),
                        Command::Execute(execution::Action::Analyze) => Some("executes it".into()),
                        _ => None,
                    };
                    let choice = Rc::new(RenderedChoice {
                        source,
                        page,
                        search: search.clone(),
                        command,
                    });
                    let pressed = choice.clone();
                    let down_view = view.clone();
                    div()
                        .id(("palette-row", i))
                        .test_support()
                        .role(Role::Button)
                        .aria_label(accessibility)
                        .when_some(description, |row, detail| row.aria_description(detail))
                        .aria_selected(Some(i) == selected)
                        .h(px(height))
                        .px_4()
                        .text_size(px(14.))
                        .line_height(px(20.))
                        .when(writing, |r| r.opacity(0.5))
                        .when(!writing, |r| {
                            r.cursor_pointer()
                                .hover(|r| r.bg(cx.theme().accent).opacity(0.8))
                        })
                        .when(Some(i) == selected, |r| r.bg(cx.theme().accent))
                        .child(content)
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            let _ = down_view.update(cx, |v, cx| {
                                if let Some(p) = &mut v.palette
                                    && pressed.current(p, cx)
                                {
                                    p.pressed = Some(pressed.clone());
                                }
                            });
                        })
                        .on_click(move |event, w, cx| {
                            let _ = view
                                .update(cx, |v, cx| v.click_palette_choice(&choice, event, w, cx));
                        })
                        .into_any_element()
                })
                .h(px(height))
                .w_full(),
            )
        };
        let panel = div()
            .id("palette-panel")
            .test_support()
            .role(Role::Dialog)
            .aria_label("Command palette")
            .occlude()
            .key_context("RsqlPalette")
            .track_focus(&p.list_focus)
            .w(px(560.))
            .max_w_full()
            .rounded(px(8.))
            .shadow(vec![
                BoxShadow::new(px(0.), px(25.), black().opacity(0.25))
                    .blur_radius(px(50.))
                    .spread_radius(px(-12.)),
            ])
            .overflow_hidden()
            .text_size(px(14.))
            .line_height(px(20.))
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .font_family(cx.theme().mono_font_family.clone())
            .border_1()
            .border_color(cx.theme().border)
            .capture_key_down(cx.listener(|v, e: &KeyDownEvent, w, cx| {
                let Some(palette) = v.palette.as_mut() else {
                    return;
                };
                palette.sync_search_selection(cx);
                let p = v.palette.as_ref().unwrap();
                if e.keystroke.modifiers.modified()
                    || p.search
                        .update(cx, |s, cx| s.marked_text_range(w, cx).is_some())
                {
                    return;
                }
                match e.keystroke.key.as_str() {
                    "escape" if p.page != Page::Root => v.page(Page::Root, w, cx),
                    "escape" => v.dismiss_palette(w, cx),
                    "backspace" if p.page != Page::Root && p.search.read(cx).value().is_empty() => {
                        v.page(Page::Root, w, cx)
                    }
                    "down" | "up" if p.workspace_operation.writing() => {}
                    "down" | "up" if !matches!(p.page, Page::SaveWorkspace | Page::SaveQuery) => {
                        let entries = v.entries(p, cx);
                        let palette = v.palette.as_mut().unwrap();
                        let selected = palette.reconcile_selection(&entries);
                        let count = entries.len();
                        if count != 0 {
                            let index = if e.keystroke.key == "up" {
                                selected.map_or(count - 1, |i| (i + count - 1) % count)
                            } else {
                                selected.map_or(0, |i| (i + 1) % count)
                            };
                            palette.choose_index(index, &entries);
                            cx.notify();
                        }
                    }
                    "enter" if p.workspace_operation.writing() => {}
                    "enter" if p.page == Page::SaveWorkspace => v.submit_workspace_save(w, cx),
                    "enter" if p.page == Page::SaveQuery => v.submit_palette_query(w, cx),
                    "enter" => {
                        let entries = v.entries(p, cx);
                        let palette = v.palette.as_mut().unwrap();
                        if let Some(index) = palette.reconcile_selection(&entries) {
                            let command = &entries[index].1;
                            v.run_command(command.clone(), w, cx);
                        }
                    }
                    _ => return,
                }
                cx.stop_propagation();
                w.prevent_default();
            }))
            .children(self.workspace_header(page, cx))
            .children((!page.workspace_list()).then(|| {
                let mut header = div()
                    .id("palette-input-chrome")
                    .test_support()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border);
                if matches!(page, Page::SaveWorkspace | Page::SaveQuery) {
                    header = header.child(workspaces::workspace_icon(IconName::Save, false, cx));
                } else if page != Page::Root {
                    header = header.child(self.palette_back(cx));
                }
                let id = match page {
                    Page::SaveWorkspace => "workspace-name",
                    Page::SaveQuery => "saved-query-title",
                    _ => "palette-search",
                };
                header
                    .child(
                        div().flex_1().min_w_0().child(Styled::h(
                            Input::new(&p.search)
                                .id(id)
                                .appearance(false)
                                .bordered(false)
                                .p_0()
                                .text_size(px(14.))
                                .line_height(px(20.))
                                .font_family(cx.theme().mono_font_family.clone()),
                            px(20.),
                        )),
                    )
                    .children(
                        matches!(page, Page::SaveWorkspace | Page::SaveQuery)
                            .then(|| self.palette_back(cx)),
                    )
            }))
            .child(rows)
            .children((page == Page::SaveQuery).then(|| {
                div()
                    .px_4()
                    .py_3()
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(cx.theme().muted_foreground)
                    .child("Press Enter to save the current query")
            }))
            .child(self.workspace_status(p, cx));
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .child(
                    div()
                        .id("palette-backdrop")
                        .bg(black().opacity(0.5))
                        .test_support()
                        .absolute()
                        .inset_0()
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|v, _, w, cx| v.dismiss_palette(w, cx)),
                        ),
                )
                .child(
                    div()
                        .absolute()
                        .top(relative(0.15))
                        .w_full()
                        .px_4()
                        .flex()
                        .justify_center()
                        .child(panel),
                ),
        )
    }
}
