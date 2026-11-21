use super::*;
use gpui_kit::component::{ActiveTheme, Disableable, button::Button, label::Label};
use theme::controls::compact_button;

const PAGE_BYTES: usize = 4096;

pub(super) struct Definition {
    full: Arc<String>,
    page: usize,
    pub(super) editor: Entity<EditorState>,
}

impl Definition {
    fn text(full: &str, page: usize) -> &str {
        let start = full.floor_char_boundary((page * PAGE_BYTES).min(full.len()));
        let end = full.floor_char_boundary(((page + 1) * PAGE_BYTES).min(full.len()));
        &full[start..end]
    }

    fn new(full: Arc<String>, w: &mut Window, cx: &mut App) -> Self {
        let editor =
            cx.new(|cx| EditorState::new(w, cx).default_value(Self::text(&full, 0).to_owned()));
        Self {
            full,
            page: 0,
            editor,
        }
    }

    fn pages(&self) -> usize {
        self.full.len().div_ceil(PAGE_BYTES).max(1)
    }
}

pub(super) struct Detail {
    pub index: usize,
    pub(super) definitions: [Option<Definition>; 2],
}

impl Panel {
    pub(super) fn select(
        &mut self,
        result: &Arc<Comparison>,
        index: usize,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.result.as_ref().is_some_and(|r| Arc::ptr_eq(r, result))
            || !result.range(self.filter).contains(&index)
        {
            return;
        }
        let entry = &result.entries[index];
        let definitions = [
            entry.left.clone(),
            (entry.status != Status::Identical)
                .then(|| entry.right.clone())
                .flatten(),
        ]
        .map(|value| value.map(|full| Definition::new(full, w, cx)));
        self.selected = Some(Detail { index, definitions });
        cx.notify();
    }

    fn page(&mut self, side: usize, page: usize, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(d) = self
            .selected
            .as_mut()
            .and_then(|s| s.definitions[side].as_mut())
            && page < d.pages()
        {
            d.page = page;
            d.editor.update(cx, |e, cx| {
                e.set_value(Definition::text(&d.full, page).to_owned(), w, cx)
            });
            cx.notify();
        }
    }

    pub(super) fn detail(&self, cx: &Context<Self>) -> Div {
        let panel = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .p_3()
            .gap_2();
        let Some((detail, result)) = self.selected.as_ref().zip(self.result.as_ref()) else {
            return panel
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child(Label::new("Select an object to view details"));
        };
        let entry = &result.entries[detail.index];
        let mut columns = div().flex().gap_2().flex_1().min_h_0();
        for (side, definition) in detail.definitions.iter().enumerate() {
            let Some(d) = definition else { continue };
            let tint = match (entry.status, side) {
                (Status::Modified, 0) => cx.theme().danger,
                (Status::Modified, _) => cx.theme().success,
                _ => cx.theme().border,
            };
            let mut column = div()
                .id(("schema-definition", side))
                .test_support()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .border_1()
                .border_color(tint.opacity(0.35))
                .bg(tint.opacity(0.05))
                .rounded_lg()
                .p_2()
                .child(
                    div()
                        .text_size(px(10.))
                        .child(Label::new(self.schemas[side].clone())),
                );
            if d.full.is_empty() {
                columns = columns.child(column.child(Label::new("No definition available")));
                continue;
            }
            if d.pages() > 1 {
                let page = d.page;
                column = column.child(
                    div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .child(
                            compact_button(Button::new(("previous-page", side)))
                                .label("‹")
                                .tooltip("Previous definition page")
                                .disabled(page == 0)
                                .on_click(cx.listener(move |p, _, w, cx| {
                                    p.page(side, page.saturating_sub(1), w, cx)
                                })),
                        )
                        .child(format!("{} / {}", page + 1, d.pages()))
                        .child(
                            compact_button(Button::new(("next-page", side)))
                                .label("›")
                                .tooltip("Next definition page")
                                .disabled(page + 1 == d.pages())
                                .on_click(
                                    cx.listener(move |p, _, w, cx| p.page(side, page + 1, w, cx)),
                                ),
                        ),
                );
            }
            columns = columns.child(
                column.child(
                    div().flex_1().min_h_0().child(
                        Editor::new(&d.editor)
                            .readonly(true)
                            .appearance(false)
                            .aria_label(format!("Definition in {}", self.schemas[side]))
                            .h_full(),
                    ),
                ),
            );
        }
        panel
            .child(
                div()
                    .id("schema-object-heading")
                    .overflow_x_scroll()
                    .child(Label::new(format!("{}  {}", entry.kind, entry.name))),
            )
            .child(columns)
    }
}
