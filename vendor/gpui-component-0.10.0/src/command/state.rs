use gpui_base::LogicalStyledExt as _;
use gpui_base::TestSupportExt as _;
use std::rc::Rc;

use gpui::{
    AbsoluteLength, AnyElement, App, AppContext as _, AvailableSpace, Context, Entity, FocusHandle,
    Focusable, FontFallbacks, FontFeatures, FontStyle, FontWeight, InteractiveElement, IntoElement,
    KeyBinding, Keystroke, ListSizingBehavior, ParentElement, Pixels, Render, Role, ScrollStrategy,
    SharedString, Size, StatefulInteractiveElement as _, StyleRefinement, Styled, Subscription,
    TextOverflow, WhiteSpace, Window, div, prelude::FluentBuilder as _, px, size,
};
use rust_i18n::t;

use crate::{
    ActiveTheme as _, ElementExt as _, Icon, IconName, IndexPath, StyledExt as _,
    VirtualListScrollHandle,
    actions::{Cancel, Confirm, SelectDown, SelectUp},
    command::{
        command::CommandOptions,
        item::{CommandEntry, CommandItem},
    },
    h_flex,
    input::{Input, InputEvent, InputState},
    kbd::Kbd,
    scroll::Scrollbar,
    v_flex, v_virtual_list,
};

pub(crate) const CONTEXT: &str = "Command";

/// The row a separator occupies: a one-pixel rule with a little air on
/// either side. Fixed, so that only the item and heading rows need measuring.
const SEPARATOR_ROW_HEIGHT: f32 = 9.;

pub(crate) type OnQuery = dyn Fn(&str, &mut Window, &mut App);
pub(crate) type OnIndex = dyn Fn(IndexPath, &mut Window, &mut App);
pub(crate) type OnCancel = dyn Fn(&mut Window, &mut App);

pub(crate) struct CommandModel {
    pub(crate) entries: Vec<CommandEntry>,
    pub(crate) searchable: bool,
    pub(crate) filterable: bool,
    pub(crate) highlight_matches: bool,
    pub(crate) on_query: Option<Rc<OnQuery>>,
    pub(crate) on_select: Option<Rc<OnIndex>>,
    pub(crate) on_confirm: Option<Rc<OnIndex>>,
    pub(crate) on_cancel: Option<Rc<OnCancel>>,
}

impl Default for CommandModel {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            searchable: true,
            filterable: true,
            highlight_matches: false,
            on_query: None,
            on_select: None,
            on_confirm: None,
            on_cancel: None,
        }
    }
}

pub(crate) fn init(cx: &mut App) {
    let context: Option<&str> = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, context),
        KeyBinding::new("enter", Confirm { secondary: false }, context),
        KeyBinding::new("up", SelectUp, context),
        KeyBinding::new("down", SelectDown, context),
    ]);
}

/// One rendered line of the list.
///
/// Groups are flattened into headings and items so the list is a single
/// sequence of rows, which is what the virtual list scrolls over.
#[derive(Clone, PartialEq)]
enum CommandRow {
    Heading(SharedString),
    /// Holds the index into [`CommandState::matched`].
    Item(usize),
    Separator,
}

#[derive(Clone, PartialEq)]
struct TextShapeKey {
    font_family: SharedString,
    font_features: FontFeatures,
    font_fallbacks: Option<FontFallbacks>,
    font_size: AbsoluteLength,
    font_weight: FontWeight,
    font_style: FontStyle,
    white_space: WhiteSpace,
    text_overflow: Option<TextOverflow>,
    line_clamp: Option<usize>,
}

#[derive(Clone, PartialEq)]
struct ListMeasurementKey {
    content_width: Pixels,
    rem_size: Pixels,
    line_height: Pixels,
    text_shape: TextShapeKey,
}

/// An item that survived the current query, and where it landed.
#[derive(Clone)]
struct MatchedItem {
    entry_ix: usize,
    item_ix: usize,
    index_path: IndexPath,
    row_ix: usize,
    disabled: bool,
}

/// The interaction state of a [`crate::command::Command`] palette: its query,
/// focus, scrolling, and highlighted command.
pub struct CommandState {
    focus_handle: FocusHandle,
    query_input: Entity<InputState>,
    scroll_handle: VirtualListScrollHandle,
    model: CommandModel,
    rows: Vec<CommandRow>,
    row_sizes: Rc<Vec<Size<Pixels>>>,
    list_measurement_key: Option<ListMeasurementKey>,
    needs_measure: bool,
    /// The keybinding hint each matched item showed when the rows were last
    /// measured. Hints are looked up on every render, so a keymap change can
    /// resize a row while the model stays the same.
    measured_bindings: Vec<Option<Keystroke>>,
    matched: Vec<MatchedItem>,
    selected_index: Option<usize>,
    preserve_no_selection: bool,
    loading: bool,
    pending_scroll: Option<usize>,
    /// The placeholder last written to the query input, so that `render` only
    /// writes when it changed — `set_placeholder` notifies, and an
    /// unconditional notify from `render` would redraw every frame.
    applied_placeholder: SharedString,
    applied_query: SharedString,
    pub(crate) options: CommandOptions,
    _subscriptions: Vec<Subscription>,
}

impl CommandState {
    /// Create an empty palette.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query_input = cx.new(|cx| InputState::new(window, cx));

        let _subscriptions =
            vec![cx.subscribe_in(&query_input, window, Self::on_query_input_event)];

        Self {
            focus_handle: cx.focus_handle(),
            query_input,
            scroll_handle: VirtualListScrollHandle::new(),
            model: CommandModel::default(),
            rows: Vec::new(),
            row_sizes: Rc::new(Vec::new()),
            list_measurement_key: None,
            needs_measure: true,
            measured_bindings: Vec::new(),
            matched: Vec::new(),
            selected_index: None,
            preserve_no_selection: false,
            loading: false,
            pending_scroll: None,
            applied_placeholder: SharedString::default(),
            applied_query: SharedString::default(),
            options: CommandOptions::default(),
            _subscriptions,
        }
    }

    /// Apply the presentation of the latest [`crate::command::Command`] render.
    pub(crate) fn set_options(&mut self, options: CommandOptions) {
        if self.options.style.text != options.style.text {
            self.needs_measure = true;
        }
        self.options = options;
    }

    pub(crate) fn install_model(&mut self, model: CommandModel, cx: &mut Context<Self>) {
        let selected_index_path = self.selected_index();
        // The model reinstalls on every host re-render, usually unchanged.
        // Keep the rows and their measured sizes then: measuring lays out
        // every row, not only the visible ones.
        let same_layout = self.model.searchable == model.searchable
            && self.model.filterable == model.filterable
            && self.model.entries.len() == model.entries.len()
            && self
                .model
                .entries
                .iter()
                .zip(&model.entries)
                .all(|(entry, other)| entry.same_layout(other));
        self.model = model;
        if !same_layout {
            self.update_matches(cx);
        }

        let preserved_selection = selected_index_path.and_then(|selected_index_path| {
            self.matched
                .iter()
                .enumerate()
                .find_map(|(matched_ix, matched)| {
                    (!matched.disabled && matched.index_path == selected_index_path)
                        .then_some(matched_ix)
                })
        });

        if let Some(matched_ix) = preserved_selection {
            // Preserving the selection is not a navigation: the model
            // reinstalls on every host re-render, so scrolling here would
            // move the list one frame after a hover selection.
            self.selected_index = Some(matched_ix);
            self.preserve_no_selection = false;
        } else if self.preserve_no_selection {
            self.selected_index = None;
            self.pending_scroll = None;
        } else {
            self.reset_selection();
        }
    }

    /// The current search query.
    pub fn query(&self, cx: &App) -> SharedString {
        self.query_input.read(cx).value()
    }

    /// Replace the search query, as if it had been typed.
    ///
    /// The input suppresses its own change event for a programmatic write, so
    /// the re-filter and query callback happen here instead.
    pub fn set_query(
        &mut self,
        query: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = query.into();
        if self.query(cx) == query {
            return;
        }

        self.query_input
            .update(cx, |input, cx| input.set_value(query, window, cx));
        self.on_query_changed(window, cx);
    }

    /// The highlighted item's path in the model installed by the latest
    /// [`crate::command::Command`] render, before local filtering.
    ///
    /// Ungrouped items occupy section 0 and use their input position as the
    /// row. Explicit groups use their group and item positions; when a model
    /// mixes both forms, the implicit ungrouped section comes first.
    pub fn selected_index(&self) -> Option<IndexPath> {
        self.selected_index
            .and_then(|selected_index| self.matched.get(selected_index))
            .filter(|matched| !matched.disabled)
            .map(|matched| matched.index_path)
    }

    /// Highlight an item by its original, unfiltered model path, or clear the
    /// highlight with `None`.
    ///
    /// A path that is currently filtered out or disabled clears the
    /// highlight. A visible selection is scrolled into view.
    pub fn set_selected_index(
        &mut self,
        index: Option<IndexPath>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let matched_ix = index.and_then(|index| {
            self.matched
                .iter()
                .position(|matched| matched.index_path == index && !matched.disabled)
        });

        let preserve_no_selection = matched_ix.is_none();
        if self.selected_index == matched_ix {
            self.preserve_no_selection = preserve_no_selection;
            return;
        }

        let previous_index = self.selected_index();
        self.selected_index = matched_ix;
        self.preserve_no_selection = preserve_no_selection;
        self.pending_scroll = matched_ix
            .and_then(|matched_ix| self.matched.get(matched_ix))
            .map(|matched| matched.row_ix);

        if let Some((on_select, index)) = self.on_select_if_changed(previous_index) {
            window.defer(cx, move |window, cx| on_select(index, window, cx));
        }

        cx.notify();
    }

    /// The number of items matching the current query.
    pub fn matched_count(&self) -> usize {
        self.matched.len()
    }

    /// Move focus to the palette's active control.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        if self.model.searchable {
            self.query_input.focus_handle(cx).focus(window, cx);
        } else {
            self.focus_handle.focus(window, cx);
        }
    }

    /// Show or hide the search field's spinner, and suppress the empty message
    /// while it spins.
    ///
    /// Turn it on while an `on_query` callback is being answered.
    pub fn set_loading(&mut self, loading: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = loading;
        self.query_input
            .update(cx, |input, cx| input.set_loading(loading, window, cx));
        cx.notify();
    }

    /// Whether the search field is showing its spinner.
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    // MARK: Matching

    fn item_matches(&self, item: &CommandItem, query: &str) -> bool {
        if !self.model.searchable || !self.model.filterable || query.is_empty() {
            true
        } else {
            item.matches(query)
        }
    }

    /// The query the rows highlight in their labels: the one the palette
    /// filtered by when `highlight_matches` is on, or none.
    fn highlighted_query(&self) -> &str {
        if self.model.highlight_matches && self.model.searchable && self.model.filterable {
            self.applied_query.trim()
        } else {
            ""
        }
    }

    fn item_at(&self, matched_ix: usize) -> Option<&CommandItem> {
        let matched = self.matched.get(matched_ix)?;

        match self.model.entries.get(matched.entry_ix)? {
            CommandEntry::Item(item) => Some(item),
            CommandEntry::Group(group) => group.items.get(matched.item_ix),
            CommandEntry::Separator => None,
        }
    }

    /// Recompute the visible rows and the matching items for the current query.
    ///
    fn update_matches(&mut self, cx: &App) {
        let query = self.query(cx);
        let query = query.trim();

        let mut rows: Vec<CommandRow> = Vec::new();
        let mut matched: Vec<MatchedItem> = Vec::new();
        let has_ungrouped_items = self
            .model
            .entries
            .iter()
            .any(|entry| matches!(entry, CommandEntry::Item(_)));
        let mut ungrouped_item_ix = 0;
        let mut group_ix = 0;
        // A separator is only drawn once something follows it, which drops the
        // leading, trailing and doubled separators a filtered list leaves behind.
        let mut pending_separator = false;

        for (entry_ix, entry) in self.model.entries.iter().enumerate() {
            match entry {
                CommandEntry::Separator => pending_separator = !rows.is_empty(),
                CommandEntry::Item(item) => {
                    let item_ix = ungrouped_item_ix;
                    ungrouped_item_ix += 1;
                    if !self.item_matches(item, query) {
                        continue;
                    }

                    if pending_separator {
                        rows.push(CommandRow::Separator);
                        pending_separator = false;
                    }

                    let index_path = IndexPath::new(item_ix).section(0);
                    matched.push(MatchedItem {
                        entry_ix,
                        item_ix: 0,
                        index_path,
                        row_ix: rows.len(),
                        disabled: item.is_disabled(),
                    });
                    rows.push(CommandRow::Item(matched.len() - 1));
                }
                CommandEntry::Group(group) => {
                    let section_ix = group_ix + usize::from(has_ungrouped_items);
                    group_ix += 1;
                    let visible = group
                        .items
                        .iter()
                        .enumerate()
                        .filter(|(_, item)| self.item_matches(item, query))
                        .map(|(item_ix, item)| (item_ix, item.is_disabled()))
                        .collect::<Vec<_>>();

                    if visible.is_empty() {
                        continue;
                    }

                    if pending_separator {
                        rows.push(CommandRow::Separator);
                        pending_separator = false;
                    }

                    if let Some(heading) = group.heading() {
                        rows.push(CommandRow::Heading(heading.clone()));
                    }

                    for (item_ix, disabled) in visible {
                        let index_path = IndexPath::new(item_ix).section(section_ix);
                        matched.push(MatchedItem {
                            entry_ix,
                            item_ix,
                            index_path,
                            row_ix: rows.len(),
                            disabled,
                        });
                        rows.push(CommandRow::Item(matched.len() - 1));
                    }
                }
            }
        }

        self.rows = rows;
        self.matched = matched;
        self.needs_measure = true;
        self.selected_index = self.selected_index.and_then(|selected_index| {
            (selected_index < self.matched.len()).then_some(selected_index)
        });
    }

    /// Move the highlight to the first item that can be confirmed.
    fn reset_selection(&mut self) {
        self.selected_index = self.matched.iter().position(|matched| !matched.disabled);
        self.preserve_no_selection = false;
        self.pending_scroll = self
            .selected_index
            .and_then(|selected_index| self.matched.get(selected_index))
            .map(|matched| matched.row_ix)
            .or(Some(0));
    }

    fn on_query_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, InputEvent::Change) {
            return;
        }

        self.on_query_changed(window, cx);
    }

    /// Re-filter for the query that is now in the field, and report it.
    fn on_query_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.query(cx);
        if query == self.applied_query {
            return;
        }

        let previous_selection = self.selected_index();
        self.applied_query = query.clone();
        self.update_matches(cx);
        self.reset_selection();
        let selection_callback = self.on_select_if_changed(previous_selection);
        let query_callback = self
            .model
            .searchable
            .then(|| self.model.on_query.clone())
            .flatten();

        if selection_callback.is_some() || query_callback.is_some() {
            window.defer(cx, move |window, cx| {
                if let Some((on_select, index)) = selection_callback {
                    on_select(index, window, cx);
                }
                if let Some(on_query) = query_callback {
                    on_query(query.as_ref(), window, cx);
                }
            });
        }

        cx.notify();
    }

    fn set_list_measurement_key(
        &mut self,
        measurement_key: ListMeasurementKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.list_measurement_key.as_ref() == Some(&measurement_key) {
            return;
        }

        self.list_measurement_key = Some(measurement_key);
        self.measure_on_next_frame(window, cx);
    }

    /// Measure the rows again on the next frame. Called while the list is
    /// drawn, where a plain notify schedules no new frame.
    fn measure_on_next_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.needs_measure = true;
        let entity_id = cx.entity_id();
        window.on_next_frame(move |_, cx| cx.notify(entity_id));
    }

    // MARK: Actions

    fn on_select_if_changed(
        &self,
        previous_index: Option<IndexPath>,
    ) -> Option<(Rc<OnIndex>, IndexPath)> {
        let index = self.selected_index();
        if index == previous_index {
            return None;
        }

        self.model.on_select.clone().zip(index)
    }

    /// Highlight an item without scrolling it into view. Hover goes through
    /// here, and revealing a half-clipped edge row would slide the next row
    /// under the resting cursor, hover-selecting and scrolling in a loop.
    fn select(&mut self, matched_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_index == Some(matched_ix) {
            return;
        }

        let previous_index = self.selected_index();
        self.selected_index = Some(matched_ix);
        self.preserve_no_selection = false;

        if let Some((on_select, index)) = self.on_select_if_changed(previous_index) {
            window.defer(cx, move |window, cx| on_select(index, window, cx));
        }

        cx.notify();
    }

    /// Move the highlight by `step` items, wrapping around and skipping the
    /// disabled ones.
    fn select_by(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.matched.len();
        if len == 0 {
            return;
        }

        let mut next = self
            .selected_index
            .unwrap_or_else(|| if step >= 0 { len.saturating_sub(1) } else { 0 });
        let mut enabled = None;
        for _ in 0..len {
            next = (next as isize + step).rem_euclid(len as isize) as usize;
            if !self.matched[next].disabled {
                enabled = Some(next);
                break;
            }
        }

        if let Some(next) = enabled
            && self.selected_index != Some(next)
        {
            self.pending_scroll = self.matched.get(next).map(|matched| matched.row_ix);
            self.select(next, window, cx);
        }
    }

    fn on_action_select_up(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        self.select_by(-1, window, cx);
    }

    fn on_action_select_down(
        &mut self,
        _: &SelectDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_by(1, window, cx);
    }

    fn on_action_confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selected_index) = self.selected_index {
            self.confirm(selected_index, window, cx);
        }
    }

    /// Escape clears a non-empty query first, and only then leaves the palette
    /// — the dialog that hosts it closes on the second press.
    fn on_action_cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.searchable && !self.query(cx).is_empty() {
            self.set_query("", window, cx);
            return;
        }

        // Cancel is the one synchronous callback: propagation must continue in
        // this dispatch so a hosting Dialog observes it once and owns the pop.
        if let Some(on_cancel) = self.model.on_cancel.clone() {
            on_cancel(window, cx);
        }

        cx.propagate();
    }

    fn confirm(&mut self, matched_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.item_at(matched_ix) else {
            return;
        };
        if item.is_disabled() {
            return;
        }

        let index_path = self.matched[matched_ix].index_path;
        let action = item.action.as_ref().map(|action| action.boxed_clone());
        let on_confirm = self.model.on_confirm.clone();

        if let Some(action) = action {
            window.dispatch_action(action, cx);
        }
        if let Some(on_confirm) = on_confirm {
            window.defer(cx, move |window, cx| {
                on_confirm(index_path, window, cx);
            });
        }
    }

    // MARK: Row sizing

    /// Measure each row before passing the sizes to the virtual list. Custom
    /// item elements can have independent intrinsic heights.
    fn measure_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let available = size(
            self.list_measurement_key
                .as_ref()
                .map_or(AvailableSpace::MinContent, |key| {
                    AvailableSpace::Definite(key.content_width)
                }),
            AvailableSpace::MinContent,
        );
        let mut text_style = StyleRefinement::default();
        text_style.text = self.options.style.text.clone();
        let mut bindings = vec![None; self.matched.len()];

        let row_sizes = self
            .rows
            .iter()
            .enumerate()
            .map(|(row_ix, row)| {
                let row = match row {
                    CommandRow::Separator => return size(px(0.), px(SEPARATOR_ROW_HEIGHT)),
                    CommandRow::Heading(_) => self.render_row(row_ix, window, cx),
                    CommandRow::Item(matched_ix) => {
                        let binding = self.item_binding(*matched_ix, window, cx);
                        bindings[*matched_ix] = binding.as_ref().map(|kbd| kbd.keystroke().clone());
                        self.render_item_with_binding(*matched_ix, binding, window, cx)
                    }
                };
                let row_size = div()
                    .refine_style(&text_style)
                    .child(row)
                    .into_any_element()
                    .layout_as_root(available, window, cx);
                size(px(0.), row_size.height)
            })
            .collect::<Vec<_>>();

        self.row_sizes = Rc::new(row_sizes);
        self.measured_bindings = bindings;
    }

    // MARK: Rendering

    fn sync_placeholder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = self
            .options
            .placeholder
            .as_ref()
            .cloned()
            .unwrap_or_else(|| t!("Command.placeholder").to_string().into());

        if self.applied_placeholder == placeholder {
            return;
        }

        self.applied_placeholder = placeholder.clone();
        self.query_input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx)
        });
    }

    /// Row corner radius, concentric with the frame: the list's `p_1` (plus the
    /// border when bordered) sits between a row and the frame's corner. A
    /// borderless command without its own radius keeps the theme radius, since
    /// its frame belongs to the host.
    fn item_radius(&self, window: &Window, cx: &App) -> Pixels {
        let theme = cx.theme();
        let rem_size = window.rem_size();
        let own_radius = self.options.style.corner_radii.top_left;
        let (outer, border) = match (self.options.bordered, own_radius) {
            (true, radius) => (
                radius.map_or(theme.radius_lg, |r| r.to_pixels(rem_size)),
                px(1.),
            ),
            (false, Some(radius)) => (radius.to_pixels(rem_size), px(0.)),
            (false, None) => return theme.radius,
        };
        (outer - rem_size * 0.25 - border).max(px(0.))
    }

    /// The frame every item row shares, so that the measured height matches the
    /// rendered one.
    fn item_row(&self, selected: bool, window: &Window, cx: &App) -> gpui::Div {
        div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .gap_2()
            .px_2()
            .py_1p5()
            .text_sm()
            .rounded(self.item_radius(window, cx))
            .when(selected, |this| {
                this.bg(cx.theme().accent)
                    .text_color(cx.theme().accent_foreground)
            })
    }

    fn heading_row(&self, heading: SharedString, cx: &App) -> gpui::Div {
        div()
            .w_full()
            .px_2()
            .py_1p5()
            .text_xs()
            .font_medium()
            .text_color(cx.theme().muted_foreground)
            .child(heading)
    }

    fn render_row(&self, row_ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.rows.get(row_ix) {
            None => div().into_any_element(),
            Some(CommandRow::Separator) => div()
                .w_full()
                .py(px(4.))
                .child(div().h(px(1.)).w_full().bg(cx.theme().border))
                .into_any_element(),
            Some(CommandRow::Heading(heading)) => {
                self.heading_row(heading.clone(), cx).into_any_element()
            }
            Some(CommandRow::Item(matched_ix)) => self.render_item(*matched_ix, window, cx),
        }
    }

    /// Render a row the virtual list shows. When an item's keybinding hint no
    /// longer matches the one it was measured with, measure the rows again.
    fn render_visible_row(
        &mut self,
        row_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(&CommandRow::Item(matched_ix)) = self.rows.get(row_ix) else {
            return self.render_row(row_ix, window, cx);
        };

        let binding = self.item_binding(matched_ix, window, cx);
        let measured = self
            .measured_bindings
            .get(matched_ix)
            .and_then(Option::as_ref);
        if !self.needs_measure && measured != binding.as_ref().map(Kbd::keystroke) {
            self.measure_on_next_frame(window, cx);
        }

        self.render_item_with_binding(matched_ix, binding, window, cx)
    }

    /// The keybinding hint an item shows in its trailing slot. A custom child
    /// owns its complete presentation, so it has none.
    fn item_binding(&self, matched_ix: usize, window: &Window, cx: &App) -> Option<Kbd> {
        let item = self.item_at(matched_ix)?;
        if item.content.is_some() {
            return None;
        }

        let action = item.action.as_ref()?;
        Kbd::binding_for_action_in(action.as_ref(), &self.focus_handle(cx), window)
            .or_else(|| Kbd::binding_for_action(action.as_ref(), None, window))
    }

    fn render_item(
        &self,
        matched_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let binding = self.item_binding(matched_ix, window, cx);
        self.render_item_with_binding(matched_ix, binding, window, cx)
    }

    fn render_item_with_binding(
        &self,
        matched_ix: usize,
        binding: Option<Kbd>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(item) = self.item_at(matched_ix) else {
            return div().into_any_element();
        };

        let disabled = item.is_disabled();
        let selected = self.selected_index == Some(matched_ix) && !disabled;
        let muted_foreground = cx.theme().muted_foreground;
        let icon_color = if selected {
            cx.theme().accent_foreground
        } else {
            muted_foreground
        };
        let content = match &item.content {
            Some(render) => render(window, cx),
            None => h_flex()
                .flex_row_dir(cx)
                .flex_1()
                .gap_2()
                .items_center()
                .when_some(item.icon.clone(), |this, icon| {
                    this.child(icon.size_4().text_color(icon_color))
                })
                .when_some(item.label_text().cloned(), |this, label| {
                    this.child(crate::label::highlight_matches(
                        label,
                        self.highlighted_query(),
                        cx,
                    ))
                })
                .into_any_element(),
        };

        self.item_row(selected, window, cx)
            .id(self.matched[matched_ix].index_path)
            .test_support()
            .role(Role::ListBoxOption)
            // The row's text is drawn by a child that isn't an accessibility
            // node, so the row names itself.
            .when_some(item.label_text().cloned(), |this, label| {
                this.aria_label(label)
            })
            .aria_selected(selected)
            .when(disabled, |this| this.text_color(muted_foreground))
            .when(!disabled, |this| {
                this.cursor_default()
                    .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
                        if *hovered {
                            this.select(matched_ix, window, cx);
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.confirm(matched_ix, window, cx);
                    }))
            })
            .child(content)
            .map(|this| match binding {
                Some(binding) => this.child(binding.ms(gpui::Length::Auto, cx)),
                // The binding owns the trailing slot, so only an item without
                // one can show its check there.
                None => this.when(item.checked, |this| {
                    this.child(crate::Sizable::xsmall(
                        Icon::new(IconName::Check).ms(gpui::Length::Auto, cx),
                    ))
                }),
            })
            .into_any_element()
    }

    fn render_empty(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        if let Some(empty) = self.options.empty.as_ref() {
            return empty(self, window, cx);
        }

        let message: SharedString = t!("Command.empty").to_string().into();

        div()
            .py_6()
            .w_full()
            .text_center()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(message)
            .into_any_element()
    }
}

impl Focusable for CommandState {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.model.searchable {
            self.query_input.focus_handle(cx)
        } else {
            self.focus_handle.clone()
        }
    }
}

impl Render for CommandState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_placeholder(window, cx);

        if self.needs_measure {
            self.needs_measure = false;
            self.measure_rows(window, cx);
        }

        if let Some(row_ix) = self.pending_scroll.take() {
            self.scroll_handle
                .scroll_to_item(row_ix, ScrollStrategy::Nearest);
        }

        let rows_count = self.rows.len();
        let row_sizes = self.row_sizes.clone();
        let command_state = cx.entity();

        v_flex()
            .id("command")
            .test_support()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_action_select_up))
            .on_action(cx.listener(Self::on_action_select_down))
            .on_action(cx.listener(Self::on_action_confirm))
            .on_action(cx.listener(Self::on_action_cancel))
            .w_full()
            .overflow_hidden()
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .when(self.options.bordered, |this| {
                this.rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
            })
            .refine_style(&self.options.style)
            .when_some(self.options.header.as_ref(), |this, header| {
                this.child(header(self, window, cx))
            })
            .when(self.model.searchable, |this| {
                this.child(
                    div()
                        .flex_none()
                        .px_3()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            Input::new(&self.query_input)
                                .prefix(
                                    Icon::new(IconName::Search)
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .appearance(false)
                                .p_0(),
                        ),
                )
            })
            .child(
                v_flex()
                    .id("command-list-container")
                    .role(Role::ListBox)
                    .relative()
                    .flex_1()
                    // The rows carry their inset on the virtual list itself so
                    // that a mid-scroll clip edge sits flush against the
                    // surrounding dividers; only the empty slot needs the
                    // container padding, and only while it is shown, so a
                    // loading list leaves no blank strip under the query.
                    .when(rows_count == 0 && !self.loading, |this| this.p_1())
                    .on_prepaint({
                        let measure_state = command_state.clone();
                        move |bounds, window, cx| {
                            measure_state.update(cx, |state, cx| {
                                // The list's `p_1` is one quarter rem on each
                                // side. Its rem-dependent padding and inherited
                                // layout-relevant text style participate in
                                // the row-size cache key.
                                let text_style = window.text_style();
                                state.set_list_measurement_key(
                                    ListMeasurementKey {
                                        content_width: (bounds.size.width
                                            - window.rem_size() * 0.5)
                                            .max(px(0.)),
                                        rem_size: window.rem_size(),
                                        line_height: window.line_height(),
                                        text_shape: TextShapeKey {
                                            font_family: text_style.font_family,
                                            font_features: text_style.font_features,
                                            font_fallbacks: text_style.font_fallbacks,
                                            font_size: text_style.font_size,
                                            font_weight: text_style.font_weight,
                                            font_style: text_style.font_style,
                                            white_space: text_style.white_space,
                                            text_overflow: text_style.text_overflow,
                                            line_clamp: text_style.line_clamp,
                                        },
                                    },
                                    window,
                                    cx,
                                )
                            })
                        }
                    })
                    .max_h(self.options.max_h)
                    .overflow_hidden()
                    // While a search is in flight the list is empty because the
                    // answer has not arrived, which is not the same as no match.
                    .when(rows_count == 0 && !self.loading, |this| {
                        this.child(self.render_empty(window, cx))
                    })
                    .when(rows_count > 0, |this| {
                        this.child(
                            v_virtual_list(
                                command_state.clone(),
                                "command-list",
                                row_sizes,
                                move |this, visible_range, window, cx| {
                                    visible_range
                                        .map(|row_ix| this.render_visible_row(row_ix, window, cx))
                                        .collect::<Vec<_>>()
                                },
                            )
                            // Padding on the virtual list acts like CSS
                            // scroll-padding: the scroll ends keep their inset
                            // while scrolled-under rows paint and clip at the
                            // list edge.
                            .p_1()
                            .with_sizing_behavior(ListSizingBehavior::Infer)
                            .track_scroll(&self.scroll_handle)
                            .map(|list| {
                                gpui_base::ScrollBounce::new(
                                    "command-scroll-bounce",
                                    &self.scroll_handle,
                                    list,
                                )
                                .when_some(self.options.scroll_bounce, |bounce, on| {
                                    bounce.enabled(on)
                                })
                            }),
                        )
                        .child(Scrollbar::vertical(&self.scroll_handle))
                    }),
            )
            .when_some(self.options.footer.as_ref(), |this, footer| {
                this.child(footer(self, window, cx))
            })
    }
}

// MARK: Tests
