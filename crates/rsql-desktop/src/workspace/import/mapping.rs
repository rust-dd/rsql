use super::*;
use gpui_kit::{
    assets::IconName,
    base::Scrollbar,
    component::{
        ActiveTheme, Icon, IndexPath, Sizable,
        label::Label,
        select::{Select, SelectDelegate, SelectEvent, SelectItem, SelectState},
        tooltip::Tooltip,
    },
};

const ROW_HEIGHT: f32 = 32.;
const ROW_GAP: f32 = 6.;

#[derive(Clone)]
pub(super) struct Destination {
    label: SharedString,
    column: Option<usize>,
}

impl SelectItem for Destination {
    type Value = Option<usize>;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.column
    }
}

#[derive(Clone, Default)]
pub(super) struct Destinations(Rc<Vec<Destination>>);

impl Destinations {
    pub(super) fn new(table: &ImportTable) -> Self {
        let skip = Destination {
            label: "-- skip --".into(),
            column: None,
        };
        Self(Rc::new(
            std::iter::once(skip)
                .chain(
                    table
                        .columns
                        .iter()
                        .enumerate()
                        .map(|(i, col)| Destination {
                            label: col.name.clone().into(),
                            column: Some(i),
                        }),
                )
                .collect(),
        ))
    }
}

impl SelectDelegate for Destinations {
    type Item = Destination;

    fn items_count(&self, _: usize) -> usize {
        self.0.len()
    }

    fn item(&self, index: IndexPath) -> Option<&Destination> {
        self.0.get(index.row)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Destination: SelectItem<Value = V>,
        V: PartialEq,
    {
        self.0.position(value)
    }
}

#[derive(IntoElement)]
struct MappingRow {
    owner: WeakEntity<Dialog>,
    request: Uuid,
    source: usize,
    label: String,
    headers: Arc<Vec<String>>,
    index: Option<IndexPath>,
    destinations: Destinations,
    enabled: bool,
}

impl RenderOnce for MappingRow {
    fn render(self, w: &mut Window, cx: &mut App) -> impl IntoElement {
        let source = self.source;
        let request = self.request;
        let key = ElementId::named_usize(request.to_string(), source);
        let state = w.use_keyed_state(key, cx, |w, cx| {
            let select = cx.new(|cx| SelectState::new(self.destinations, self.index, w, cx));
            let owner = self.owner.clone();
            let trigger = select.focus_handle(cx);
            let focus = cx.on_focus(&trigger.clone(), w, move |_, w, cx| {
                if w.last_input_was_keyboard() && trigger.is_focused(w) {
                    let _ = owner.update(cx, |dialog, cx| {
                        if dialog.request == request
                            && dialog.phase == Phase::Ready
                            && dialog.current()
                        {
                            dialog.reveal_mapping_row(source);
                            cx.notify();
                        }
                    });
                }
            });
            let event = cx.subscribe(
                &select,
                move |_, _, event: &SelectEvent<Destinations>, cx| {
                    let SelectEvent::Confirm(Some(column)) = event else {
                        return;
                    };
                    let _ = self.owner.update(cx, |dialog, cx| {
                        if dialog.request == request
                            && dialog.phase == Phase::Ready
                            && dialog.current()
                            && let Some(preview) = &mut dialog.preview
                        {
                            preview.set(source, *column);
                            cx.notify();
                        }
                    });
                },
            );
            (select, event, focus)
        });
        let theme = cx.theme();
        let control = Select::new(&state.read(cx).0)
            .id(("csv-map", source))
            .accessibility_label(format!(
                "Destination for column {} ({})",
                source + 1,
                self.label
            ))
            .placeholder("Choose target")
            .disabled(!self.enabled)
            .small()
            .h(px(26.))
            .px_2()
            .text_size(px(12.))
            .rounded(px(8.))
            .bg(theme.input.opacity(0.8))
            .border_color(theme.border.opacity(0.5));
        div()
            .id(("csv-map-row", source))
            .flex()
            .items_center()
            .gap_2()
            .h(px(ROW_HEIGHT))
            .pb(px(ROW_GAP))
            .child(
                div()
                    .id(("csv-map-label", source))
                    .w(px(140.))
                    .min_w_0()
                    .truncate()
                    .child(
                        Label::new(self.label)
                            .text_color(theme.muted_foreground)
                            .line_height(px(16.)),
                    )
                    // The source title exposed the exact header; read it only when hovered.
                    .tooltip(move |w, cx| Tooltip::new(self.headers[source].clone()).build(w, cx)),
            )
            .child(
                Icon::new(IconName::ArrowRight)
                    .size(px(12.))
                    .flex_shrink_0()
                    .text_color(theme.muted_foreground.opacity(0.5)),
            )
            .child(div().flex_1().min_w_0().child(control))
    }
}

impl Dialog {
    fn reveal_mapping_row(&self, source: usize) {
        let list = self.mapping_scroll.0.borrow();
        let scroll = &list.base_handle;
        let bounds = scroll.bounds();
        let height = px(ROW_HEIGHT);
        let top = height * source;
        // Keep the previous and next native tab stops mounted as focus moves.
        let previous = (top - height).max(px(0.));
        let next_end = top + height * 2. - px(ROW_GAP);
        let offset = scroll.offset();
        let y = offset
            .y
            .min(bounds.size.height - next_end)
            .max(-previous)
            .clamp(-scroll.max_offset().y, px(0.));
        scroll.set_offset(point(offset.x, y));

        let row_top = bounds.top() + top + y;
        let viewport = self.body_scroll.bounds();
        let visible_top = row_top
            .min(viewport.bottom() - height + px(ROW_GAP))
            .max(viewport.top());
        let offset = self.body_scroll.offset();
        let y = (offset.y + visible_top - row_top).clamp(-self.body_scroll.max_offset().y, px(0.));
        self.body_scroll.set_offset(point(offset.x, y));
    }

    pub(super) fn mapping_list(&self, preview: &Preview, cx: &Context<Self>) -> Div {
        let count = preview.labels.len();
        let owner = cx.entity();
        let rows = uniform_list("csv-mapping", count, move |range, _, cx| {
            let dialog = owner.read(cx);
            let Some(preview) = &dialog.preview else {
                return Vec::new();
            };
            range
                .filter_map(|source| {
                    let column = *preview.selection.get(source)?;
                    let index = (!preview.ambiguous.contains(&source))
                        .then(|| IndexPath::default().row(column.map_or(0, |i| i + 1)));
                    Some(MappingRow {
                        owner: owner.downgrade(),
                        request: dialog.request,
                        source,
                        label: preview.labels[source].clone(),
                        headers: preview.headers.clone(),
                        destinations: dialog.destinations.clone(),
                        index,
                        enabled: dialog.phase == Phase::Ready && dialog.current(),
                    })
                })
                .collect()
        });
        let height = count.saturating_mul(32).saturating_sub(6).min(200);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .id("csv-mapping-heading")
                    .role(Role::Heading)
                    .aria_label("Column Mapping")
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(cx.theme().muted_foreground)
                    .child("COLUMN MAPPING"),
            )
            .child(
                div()
                    .id("csv-mapping-rows")
                    .relative()
                    .h(px(height as f32))
                    .child(rows.size_full().track_scroll(&self.mapping_scroll))
                    .child(Scrollbar::vertical(&self.mapping_scroll)),
            )
    }
}
