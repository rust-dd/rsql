use std::{cell::RefCell, rc::Rc};

use gpui::{
    AnyElement, App, Axis, Bounds, ClickEvent, Div, Element, ElementId, Entity, FocusHandle,
    GlobalElementId, InspectorElementId, InteractiveElement, Interactivity, IntoElement,
    KeyDownEvent, LayoutId, MouseButton, ParentElement, Pixels, Refineable as _, RenderOnce, Role,
    SharedString, StatefulInteractiveElement, StyleRefinement, Styled, Window, accesskit, div,
    prelude::FluentBuilder as _, relative,
};
use smallvec::SmallVec;

use crate::{
    LayoutDirection, StateStyle, StyledExt as _, TestSupportExt as _,
    a11y_disabled::A11yDisabledExt as _,
};

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// An unstyled tab that owns pointer activation and accessibility behavior.
///
/// Inside a [`Tabs`] list the tab also takes part in the list's roving
/// keyboard focus: it is focusable, Enter and Space activate it like a click,
/// and the list moves focus between its tabs with the arrow keys. A tab
/// rendered outside a [`Tabs`] list is pointer-only, as before.
#[derive(IntoElement)]
pub struct Tab {
    id: ElementId,
    base: Div,
    style: StyleRefinement,
    semantic_styles: TabStyles,
    selected: bool,
    disabled: bool,
    children: SmallVec<[AnyElement; 2]>,
    on_click: Option<ClickHandler>,
    accessibility_label: Option<SharedString>,
    position_in_set: Option<usize>,
    size_of_set: Option<usize>,
}

impl Tab {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            base: div(),
            style: StyleRefinement::default(),
            semantic_styles: TabStyles::default(),
            selected: false,
            disabled: false,
            children: SmallVec::new(),
            on_click: None,
            accessibility_label: None,
            position_in_set: None,
            size_of_set: None,
        }
    }

    /// Updates the element identity used when the tab is rendered.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }

    /// Sets this tab's one-based position and the tab list's total size, so
    /// assistive technology can announce "tab 2 of 5".
    pub fn set_position(mut self, position: usize, size: usize) -> Self {
        self.position_in_set = Some(position);
        self.size_of_set = Some(size);
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    pub fn styles(mut self, build: impl FnOnce(TabStyles) -> TabStyles) -> Self {
        self.semantic_styles = build(self.semantic_styles);
        self
    }

    fn resolved_style(&self) -> StyleRefinement {
        crate::state_style::resolve_style(
            &self.style,
            [
                self.selected.then_some(&self.semantic_styles.selected),
                self.disabled.then_some(&self.semantic_styles.disabled),
            ]
            .into_iter()
            .flatten(),
        )
    }
}

#[derive(Default)]
pub struct TabStyles {
    selected: StyleRefinement,
    disabled: StyleRefinement,
}

impl TabStyles {
    pub fn selected(mut self, build: impl FnOnce(StateStyle) -> StateStyle) -> Self {
        self.selected
            .refine(&build(StateStyle::default()).into_refinement());
        self
    }

    pub fn disabled(mut self, build: impl FnOnce(StateStyle) -> StateStyle) -> Self {
        self.disabled
            .refine(&build(StateStyle::default()).into_refinement());
        self
    }
}

impl Styled for Tab {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Tab {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl InteractiveElement for Tab {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Tab {}

impl RenderOnce for Tab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let disabled = self.disabled;
        let style = self.resolved_style();

        // Join the enclosing tab list, if any. The handle is keyed by the
        // tab's id so focus survives rerenders and controlled selection
        // changes; the list decides which tab is the single tab stop.
        let focus_handle = TabListScope::current().map(|registry| {
            let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
                cx.focus_handle().tab_stop(false)
            });
            let handle = state.read(cx).clone();
            registry.borrow_mut().push(TabEntry {
                state,
                focus_handle: handle.clone(),
                selected: self.selected,
                disabled,
            });
            handle
        });

        self.base
            .id(self.id)
            .test_support()
            .when_some(focus_handle, |this, handle| this.track_focus(&handle))
            .role(Role::Tab)
            // Match Button's neutral control geometry: a fixed-size tab
            // centers ordinary content, while callers still own its size,
            // spacing and visual treatment.
            .flex()
            .items_center()
            .justify_center()
            .line_height(relative(1.))
            .when_some(self.accessibility_label, |this, label| {
                this.aria_label(label)
            })
            .aria_selected(self.selected)
            .when_some(self.position_in_set, |this, position| {
                this.aria_position_in_set(position)
            })
            .when_some(self.size_of_set, |this, size| this.aria_size_of_set(size))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .when_some(
                (!disabled).then_some(self.on_click).flatten(),
                |this, on_click| {
                    this.on_click(move |event, window, cx| on_click(event, window, cx))
                },
            )
            .children(self.children)
            .refine_style(&style)
            .a11y_disabled(disabled)
    }
}

/// An unstyled collection root for [`Tab`] elements.
///
/// Controlled selection and activation stay with the child tabs: the
/// application decides which tab is selected and what activating one does.
/// The root supplies the tab-list semantics and the keyboard contract of the
/// ARIA tabs pattern (manual activation):
///
/// - The list is one tab stop. `Tab` enters it on the selected tab (the first
///   enabled tab when none is selected) and leaves it with the next `Tab`.
/// - Left and Right move focus to the previous or next enabled tab, wrapping
///   at either end; in a right-to-left layout the next tab sits to the left.
///   A vertical list ([`Tabs::axis`]) uses Up and Down instead.
/// - Home and End move focus to the first and last enabled tab.
/// - Enter and Space activate the focused tab, running its click handler.
/// - Disabled tabs are skipped and never take focus from the keyboard.
///
/// Moving focus does not select: selection follows activation, so a
/// controlled list never changes behind the application's back.
#[derive(IntoElement)]
pub struct Tabs {
    id: ElementId,
    base: gpui::Stateful<Div>,
    style: StyleRefinement,
    axis: Axis,
    children: SmallVec<[AnyElement; 2]>,
}

impl Tabs {
    pub fn new(id: impl Into<ElementId>) -> Self {
        let id = id.into();
        Self {
            base: div().id(id.clone()),
            style: StyleRefinement::default(),
            axis: Axis::Horizontal,
            children: SmallVec::new(),
            id,
        }
    }

    /// The direction the tabs run: horizontal (Left/Right, the default) or
    /// vertical (Up/Down). Assistive technology hears the same orientation.
    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }
}

/// A tab registered with its list during the list's layout, in tree order.
struct TabEntry {
    /// Holds the handle the tab tracks; its tab-stop flag is what the
    /// window's tab order reads when the tab next paints.
    state: Entity<FocusHandle>,
    focus_handle: FocusHandle,
    selected: bool,
    disabled: bool,
}

type TabRegistry = Rc<RefCell<Vec<TabEntry>>>;

thread_local! {
    static TAB_LIST_STACK: RefCell<Vec<TabRegistry>> = const { RefCell::new(Vec::new()) };
}

/// Wraps a tab list so the tabs rendered while it lays out register with it.
///
/// `Tabs` holds its children as erased elements, and each `Tab` renders
/// lazily while its parent lays out. Publishing the list's registry for the
/// duration of that layout lets the tabs join it without a typed child API,
/// so tabs nested in wrappers or lists still count, and the registry always
/// matches what was rendered last.
struct TabListScope {
    registry: TabRegistry,
    inner: AnyElement,
}

impl TabListScope {
    fn current() -> Option<TabRegistry> {
        TAB_LIST_STACK.with(|stack| stack.borrow().last().cloned())
    }
}

#[cfg(not(feature = "gpui-fast"))]
impl IntoElement for TabListScope {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TabListScope {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.registry.borrow_mut().clear();
        TAB_LIST_STACK.with(|stack| stack.borrow_mut().push(self.registry.clone()));
        let layout_id = self.inner.request_layout(window, cx);
        TAB_LIST_STACK.with(|stack| stack.borrow_mut().pop());
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        sync_tab_stops(&self.registry.borrow(), window, cx);
        self.inner.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner.paint(window, cx);
    }
}

/// Make exactly one enabled tab the list's tab stop: the focused tab while
/// the list has focus, otherwise the selected tab, otherwise the first.
fn sync_tab_stops(entries: &[TabEntry], window: &mut Window, cx: &mut App) {
    let enabled = |entry: &&TabEntry| !entry.disabled;
    let stop = entries
        .iter()
        .filter(enabled)
        .find(|entry| entry.focus_handle.is_focused(window))
        .or_else(|| entries.iter().filter(enabled).find(|entry| entry.selected))
        .or_else(|| entries.iter().find(|entry| !entry.disabled))
        .map(|entry| entry.focus_handle.clone());

    let mut changed = false;
    for entry in entries {
        let is_stop = stop.as_ref() == Some(&entry.focus_handle);
        entry.state.update(cx, |handle, _| {
            if handle.tab_stop != is_stop {
                *handle = handle.clone().tab_stop(is_stop);
                changed = true;
            }
        });
    }
    // Tabs pick up their new flag when they next render.
    if changed {
        window.refresh();
    }
}

enum Step {
    Next,
    Previous,
    First,
    Last,
}

fn handle_key_down(
    axis: Axis,
    registry: &TabRegistry,
    event: &KeyDownEvent,
    window: &mut Window,
    cx: &mut App,
) {
    let modifiers = &event.keystroke.modifiers;
    if modifiers.modified() {
        return;
    }
    let rtl = LayoutDirection::global(cx).is_rtl();
    let step = match (axis, event.keystroke.key.as_str()) {
        (Axis::Horizontal, "left") if rtl => Step::Next,
        (Axis::Horizontal, "left") => Step::Previous,
        (Axis::Horizontal, "right") if rtl => Step::Previous,
        (Axis::Horizontal, "right") => Step::Next,
        (Axis::Vertical, "up") => Step::Previous,
        (Axis::Vertical, "down") => Step::Next,
        (_, "home") => Step::First,
        (_, "end") => Step::Last,
        _ => return,
    };

    let target = {
        let entries = registry.borrow();
        let Some(current) = entries
            .iter()
            .position(|entry| entry.focus_handle.is_focused(window))
        else {
            return;
        };
        let enabled: Vec<usize> = (0..entries.len())
            .filter(|&ix| !entries[ix].disabled)
            .collect();
        let (Some(&first), Some(&last)) = (enabled.first(), enabled.last()) else {
            return;
        };
        let target = match step {
            Step::First => first,
            Step::Last => last,
            Step::Next => enabled
                .iter()
                .copied()
                .find(|&ix| ix > current)
                .unwrap_or(first),
            Step::Previous => enabled
                .iter()
                .rev()
                .copied()
                .find(|&ix| ix < current)
                .unwrap_or(last),
        };
        entries[target].focus_handle.clone()
    };

    window.focus(&target, cx);
    sync_tab_stops(&registry.borrow(), window, cx);
    cx.stop_propagation();
}

impl Styled for Tabs {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Tabs {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl InteractiveElement for Tabs {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Tabs {}

impl RenderOnce for Tabs {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            base,
            style,
            axis,
            children,
        } = self;

        // The registry outlives the frame so the key handler sees the tabs
        // rendered last, in the same order the user sees them.
        let registry = window
            .use_keyed_state(id, cx, |_, _| TabRegistry::default())
            .read(cx)
            .clone();
        let key_handler = {
            let registry = registry.clone();
            move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                handle_key_down(axis, &registry, event, window, cx);
            }
        };

        TabListScope {
            registry,
            inner: tab_list(base, axis)
                .on_key_down(key_handler)
                .children(children)
                .refine_style(&style)
                .into_any_element(),
        }
    }
}

/// The tab list's own semantics: its role and the orientation that tells
/// assistive technology which arrow keys move between the tabs.
fn tab_list(base: gpui::Stateful<Div>, axis: Axis) -> gpui::Stateful<Div> {
    base.role(Role::TabList).aria_orientation(match axis {
        Axis::Horizontal => accesskit::Orientation::Horizontal,
        Axis::Vertical => accesskit::Orientation::Vertical,
    })
}
