use super::*;
use gpui_kit::component::label::Label;
use rsql_desktop::completion::SuggestionKind;

impl Menu {
    pub(super) fn preview(
        &self,
        state: &EditorState,
        cx: &App,
    ) -> Option<gpui_kit::base::input::InlinePreview> {
        let Rows::Suggestions { items, .. } = &*self.rows else {
            return None;
        };
        let item = items.get(self.selected)?;
        let cursor = state.cursor();
        if !state.selected_range().is_empty()
            || !item.range.contains(&cursor) && item.range.end != cursor
        {
            return None;
        }
        let insertion = if item.is_snippet {
            Snippet::expand(&item.insert_text, 0).0
        } else {
            item.insert_text.clone()
        };
        Some(gpui_kit::base::input::InlinePreview {
            range: item.range.clone(),
            text: insertion.into(),
            color: cx.theme().muted_foreground.opacity(0.65),
        })
    }
}

impl Rows {
    fn prefix(&self) -> &str {
        match self {
            Self::Suggestions { prefix, .. } => prefix,
            Self::References(_) => "",
        }
    }

    fn row(&self, i: usize) -> (&str, Option<&str>, &'static str, Option<SuggestionKind>) {
        match self {
            Self::Suggestions { items, .. } => {
                let item = &items[i];
                let icon = match item.kind {
                    SuggestionKind::Column => "icons/columns-3.svg",
                    SuggestionKind::Relation => "icons/table.svg",
                    SuggestionKind::Function => "icons/parentheses.svg",
                    SuggestionKind::Schema => "icons/database.svg",
                    SuggestionKind::Keyword => "icons/key-round.svg",
                    SuggestionKind::Snippet => "icons/code.svg",
                };
                (&item.label, item.detail.as_deref(), icon, Some(item.kind))
            }
            Self::References(rows) => (&rows[i].0, None, "icons/link.svg", None),
        }
    }
}

fn prefix_end(label: &str, folded: &str) -> Option<usize> {
    if folded.is_empty() {
        return Some(0);
    }
    if !label.to_lowercase().starts_with(folded) {
        return None;
    }
    let mut consumed = 0;
    for (offset, ch) in label.char_indices() {
        consumed += ch.to_lowercase().map(char::len_utf8).sum::<usize>();
        if consumed >= folded.len() {
            return Some(offset + ch.len_utf8());
        }
    }
    None
}

fn matched_label(label: &str, prefix: &str, color: Hsla) -> StyledText {
    let text = StyledText::new(label.to_owned());
    if !prefix.is_empty()
        && let Some(end) = prefix_end(label, prefix)
    {
        text.with_highlights([(
            0..end,
            HighlightStyle {
                color: Some(color),
                ..Default::default()
            },
        )])
    } else {
        text
    }
}

impl Workspace {
    pub(super) fn language_menu(
        &self,
        state: &EditorState,
        menu: &Menu,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let bounds = state.input_bounds();
        let (caret, line) = state.cursor_layout().unwrap_or_default();
        let width = px(350.).min(bounds.size.width.max(px(1.)));
        let selected = menu.selected;
        let documentation = match &*menu.rows {
            Rows::Suggestions { items, .. } => items[selected].documentation.as_deref(),
            Rows::References(_) => None,
        };
        let available = (bounds.size.height - px(2.)).max(px(0.));
        let doc_height = if documentation.is_some() {
            px(60.).min(available * 0.4)
        } else {
            px(0.)
        };
        let list_height = px(22. * menu.len().min(8) as f32).min(available - doc_height);
        let height = (list_height + doc_height + px(2.)).min(bounds.size.height);
        let mut position = caret.origin - bounds.origin;
        position.y += state.scroll_offset().y;
        let x = position
            .x
            .clamp(px(0.), (bounds.size.width - width).max(px(0.)));
        let y = if position.y + line + height <= bounds.size.height {
            position.y + line
        } else {
            position.y - height
        };
        let colors = theme::SuggestionColors::new(cx.theme().mode.is_dark());
        let stamp = menu.stamp.clone();
        let rows = menu.rows.clone();
        let view = cx.weak_entity();
        let list = uniform_list("sql-language-items", menu.len(), move |range, _, _| {
            range
                .map(|i| {
                    let (label, detail, icon, kind) = rows.row(i);
                    let is_selected = i == selected;
                    let stamp = stamp.clone();
                    let view = view.clone();
                    let displayed = rows.clone();
                    let mut row = div()
                        .id(("sql-language-row", i))
                        .test_support()
                        .h(px(22.))
                        .px_2()
                        .flex()
                        .gap_2()
                        .items_center()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(
                            svg()
                                .path(icon)
                                .size(px(14.))
                                .flex_shrink_0()
                                .text_color(colors.icon(kind, is_selected)),
                        )
                        .child(matched_label(
                            label,
                            rows.prefix(),
                            if is_selected {
                                colors.selected_highlight
                            } else {
                                colors.highlight
                            },
                        ))
                        .children(
                            detail
                                .map(|d| div().text_color(colors.description).child(d.to_owned())),
                        )
                        .on_mouse_down(MouseButton::Left, |_, w, _| w.prevent_default())
                        .on_click(move |_, w, cx| {
                            let _ = view.update(cx, |v, cx| {
                                if v.editors
                                    .get(&stamp.owner)
                                    .and_then(|l| l.language.menu.as_ref())
                                    .is_some_and(|m| Rc::ptr_eq(&m.rows, &displayed))
                                {
                                    v.accept(&stamp, i, false, w, cx);
                                }
                            });
                        });
                    if i == selected {
                        row = row
                            .bg(colors.selected_background)
                            .text_color(colors.selected_foreground);
                    }
                    row
                })
                .collect()
        })
        .w_full()
        .h(list_height)
        .track_scroll(&menu.scroll);
        div()
            .id("sql-language-menu")
            .test_support()
            .absolute()
            .left(x)
            .top(y.clamp(px(0.), (bounds.size.height - height).max(px(0.))))
            .w(width)
            .h(height)
            .occlude()
            .bg(colors.background)
            .border_1()
            .border_color(colors.border)
            .text_color(colors.foreground)
            .text_size(px(13.))
            .font_family(cx.theme().mono_font_family.clone())
            .font_features(FontFeatures::disable_ligatures())
            .overflow_hidden()
            .child(list)
            .children(documentation.map(|doc| {
                div()
                    .id(("sql-suggestion-documentation", selected))
                    .test_support()
                    .h(doc_height)
                    .overflow_y_scroll()
                    .border_t_1()
                    .border_color(colors.border)
                    .px_2()
                    .on_mouse_down(MouseButton::Left, |_, w, _| w.prevent_default())
                    .child(
                        Label::new(doc.to_owned())
                            .text_color(colors.foreground)
                            .text_size(px(12.)),
                    )
            }))
    }
}
