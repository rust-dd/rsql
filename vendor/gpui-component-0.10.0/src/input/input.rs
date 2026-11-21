use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, DefiniteLength, Edges, ElementId, Entity, Global, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Rems, RenderOnce, Role,
    SharedString, StyleRefinement, Styled, TextAlign, TouchPhase, Window, div, px, relative,
};

use crate::button::{Button, ButtonRounded, ButtonVariants as _};
use crate::input::clear_button;
use crate::native_menu::NativeMenu;
use crate::spinner::Spinner;
use crate::touch_selection::{EditMenuItem, TouchSelectionOverlay};
use crate::{ActiveTheme, Colorize, v_flex};
use crate::{IconName, Size};
use crate::{RoleOverride, Selectable, StyledExt, h_flex};
use crate::{Sizable, StyleSized};
use gpui_base::InputBase as BaseInput;
use gpui_base::{LayoutDirection, LogicalStyledExt as _};
use rust_i18n::t;

use super::state::{TextInputState, sync_focused_input_registry};
use super::{InputContentType, InputState, sync_native_content_type};
use crate::ThemeStyled as _;

/// Set when the platform draws the touch selection UI of a focused input.
#[derive(Default)]
struct NativeTouchSelection(bool);
impl Global for NativeTouchSelection {}

/// Let the platform draw the touch selection UI of a focused [`Input`]: the
/// selection handles and the edit menu a long press leaves behind.
///
/// Use it on a host that already shows its own text UI over the window, such
/// as Android's floating toolbar and drag handles. The long press still
/// selects a word; the platform follows that selection and drives it back
/// through the window's input handler. Selections in plain text, outside an
/// input, keep the handles and menu drawn here.
pub fn set_native_touch_selection(enabled: bool, cx: &mut App) {
    cx.set_global(NativeTouchSelection(enabled));
    cx.refresh_windows();
}

fn is_native_touch_selection(cx: &App) -> bool {
    cx.try_global::<NativeTouchSelection>()
        .is_some_and(|native| native.0)
}

fn accessibility_role(
    is_multi_line: bool,
    content_type: Option<InputContentType>,
    role: RoleOverride,
) -> Option<Role> {
    role.resolve(|| {
        if is_multi_line {
            return Role::MultilineTextInput;
        }

        match content_type {
            None => Role::TextInput,
            Some(InputContentType::TelephoneNumber) => Role::PhoneNumberInput,
            Some(InputContentType::EmailAddress) => Role::EmailInput,
            Some(InputContentType::Url) => Role::UrlInput,
            Some(InputContentType::Password | InputContentType::NewPassword) => Role::PasswordInput,
            Some(InputContentType::DateTime) => Role::DateTimeInput,
            Some(InputContentType::Birthdate) => Role::DateInput,
            Some(
                InputContentType::Name
                | InputContentType::NamePrefix
                | InputContentType::GivenName
                | InputContentType::MiddleName
                | InputContentType::FamilyName
                | InputContentType::NameSuffix
                | InputContentType::Nickname
                | InputContentType::JobTitle
                | InputContentType::OrganizationName
                | InputContentType::Location
                | InputContentType::FullStreetAddress
                | InputContentType::StreetAddressLine1
                | InputContentType::StreetAddressLine2
                | InputContentType::AddressCity
                | InputContentType::AddressState
                | InputContentType::AddressCityAndState
                | InputContentType::Sublocality
                | InputContentType::CountryName
                | InputContentType::PostalCode
                | InputContentType::CreditCardNumber
                | InputContentType::CreditCardName
                | InputContentType::CreditCardGivenName
                | InputContentType::CreditCardMiddleName
                | InputContentType::CreditCardFamilyName
                | InputContentType::CreditCardSecurityCode
                | InputContentType::CreditCardExpiration
                | InputContentType::CreditCardExpirationMonth
                | InputContentType::CreditCardExpirationYear
                | InputContentType::CreditCardType
                | InputContentType::Username
                | InputContentType::OneTimeCode
                | InputContentType::ShipmentTrackingNumber
                | InputContentType::FlightNumber
                | InputContentType::BirthdateDay
                | InputContentType::BirthdateMonth
                | InputContentType::BirthdateYear
                | InputContentType::CellularEid
                | InputContentType::CellularImei,
            ) => Role::TextInput,
        }
    })
}

fn exposes_accessibility_value(content_type: Option<InputContentType>) -> bool {
    !matches!(
        content_type,
        Some(InputContentType::Password | InputContentType::NewPassword)
    )
}

/// Returns `(background, foreground)` colors for input-like components.
pub(crate) fn input_style(disabled: bool, cx: &App) -> (Hsla, Hsla) {
    if disabled {
        (
            cx.theme().input.mix_oklab(cx.theme().transparent, 0.8),
            cx.theme().muted_foreground,
        )
    } else {
        (cx.theme().input_background(), cx.theme().foreground)
    }
}

/// A text input element bind to an [`InputState`].
#[derive(IntoElement)]
pub struct Input {
    token_renderer: Option<gpui_base::input::InlineTokenRenderer>,
    token_click_listener: Option<gpui_base::input::InlineTokenClickListener>,
    token_hover_listener: Option<gpui_base::input::InlineTokenHoverListener>,
    id: Option<ElementId>,
    state: TextInputState,
    style: StyleRefinement,
    size: Size,
    prefix: Option<AnyElement>,
    suffix: Option<AnyElement>,
    height: Option<DefiniteLength>,
    appearance: bool,
    cleanable: bool,
    mask_toggle: bool,
    disabled: bool,
    readonly: bool,
    bordered: bool,
    focus_bordered: bool,
    tab_index: isize,
    selected: bool,
    content_type: Option<InputContentType>,
    role: RoleOverride,
    accessibility_id: Option<SharedString>,
    aria_label: Option<SharedString>,

    /// An optional context menu builder to allow a custom context menu on the input.
    ///
    /// If set, this overrides the built-in context menu.
    context_menu_builder: Option<Rc<dyn Fn(NativeMenu, &mut Window, &mut App) -> NativeMenu>>,

    /// An optional paste handler. If set, it is invoked with the clipboard item
    /// before the default text insertion. Return `true` if handled.
    paste_handler: Option<Rc<dyn Fn(&gpui::ClipboardItem, &mut Window, &mut App) -> bool>>,
}

impl Sizable for Input {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl Selectable for Input {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl crate::FocusableExt for Input {
    fn focus_ring(mut self, enabled: bool) -> Self {
        self.focus_bordered = enabled;
        self
    }

    fn is_focus_ring_enabled(&self) -> bool {
        self.focus_bordered
    }
}

impl Input {
    /// The element each atomic inline token renders as, in place of the default
    /// [`InputToken`](super::InputToken); editing and history stay
    /// with the input.
    pub fn token<R: IntoElement>(
        mut self,
        render: impl Fn(&super::InlineTokenContext, &mut Window, &mut App) -> R + 'static,
    ) -> Self {
        self.token_renderer = Some(Rc::new(move |token, window, cx| {
            render(token, window, cx).into_any_element()
        }));
        self
    }
    /// Open a reference after a completed, unconsumed token click.
    pub fn on_token_click(
        mut self,
        listener: impl Fn(&super::InlineTokenClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.token_click_listener = Some(Rc::new(listener));
        self
    }
    /// Report pointer presence over a token so the application can show a
    /// tooltip or run custom logic. Hover never selects or edits.
    pub fn on_token_hover(
        mut self,
        listener: impl Fn(&super::InlineTokenHoverEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.token_hover_listener = Some(Rc::new(listener));
        self
    }

    /// Sets the GPUI identity of the input frame. By default it uses the state entity ID.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Create a new [`Input`] element bind to the [`InputState`].
    pub fn new(state: &Entity<InputState>) -> Self {
        Self::with_state(state.clone().into())
    }

    /// Builds an input renderer around a state of any kind.
    ///
    /// `Textarea` and `Editor` render through this. Application code uses
    /// [`Input::new`], [`super::Textarea`], or [`super::Editor`].
    pub(crate) fn from_state(state: impl Into<TextInputState>) -> Self {
        Self::with_state(state.into())
    }

    fn with_state(state: TextInputState) -> Self {
        Self {
            id: None,
            state,
            size: Size::default(),
            style: StyleRefinement::default(),
            prefix: None,
            suffix: None,
            height: None,
            appearance: true,
            cleanable: false,
            mask_toggle: false,
            disabled: false,
            readonly: false,
            bordered: true,
            focus_bordered: true,
            tab_index: 0,
            selected: false,
            content_type: None,
            role: RoleOverride::default(),
            accessibility_id: None,
            aria_label: None,
            context_menu_builder: None,
            paste_handler: None,
            token_renderer: None,
            token_click_listener: None,
            token_hover_listener: None,
        }
    }

    /// Set the developer-assigned identifier exposed to accessibility clients.
    /// The state this input renders; a compound control reads focus and
    /// presentation from it.
    pub(crate) fn state(&self) -> &TextInputState {
        &self.state
    }

    pub(crate) fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn accessibility_id(mut self, id: impl Into<SharedString>) -> Self {
        self.accessibility_id = Some(id.into());
        self
    }

    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }

    pub fn prefix(mut self, prefix: impl IntoElement) -> Self {
        self.prefix = Some(prefix.into_any_element());
        self
    }

    pub fn suffix(mut self, suffix: impl IntoElement) -> Self {
        self.suffix = Some(suffix.into_any_element());
        self
    }

    /// Set full height of the input (Multi-line only).
    pub fn h_full(mut self) -> Self {
        self.height = Some(relative(1.));
        self
    }

    /// Set height of the input (Multi-line only).
    pub fn h(mut self, height: impl Into<DefiniteLength>) -> Self {
        self.height = Some(height.into());
        self
    }

    /// Set the appearance of the input field, if false the input field will no border, background.
    pub fn appearance(mut self, appearance: bool) -> Self {
        self.appearance = appearance;
        self
    }

    /// Set the bordered for the input, default: true
    pub fn bordered(mut self, bordered: bool) -> Self {
        self.bordered = bordered;
        self
    }

    /// Set focus border for the input, default is true.
    pub fn focus_bordered(mut self, bordered: bool) -> Self {
        self.focus_bordered = bordered;
        self
    }

    /// Set whether to show the clear button when the input field is not empty, default is false.
    pub fn cleanable(mut self, cleanable: bool) -> Self {
        self.cleanable = cleanable;
        self
    }

    /// Set to enable toggle button for password mask state.
    pub fn mask_toggle(mut self) -> Self {
        self.mask_toggle = true;
        self
    }

    /// Set the semantic content type for password managers and autofill.
    ///
    /// This is a component-level semantic hint. It does not change the text
    /// value or masked rendering state.
    pub fn content_type(mut self, content_type: InputContentType) -> Self {
        self.content_type = Some(content_type);
        self
    }

    /// Override the accessible role for the input.
    ///
    /// If unset, the role is inferred from multi-line mode and content type.
    pub fn role(mut self, role: impl Into<RoleOverride>) -> Self {
        self.role = role.into();
        self
    }

    /// Set to disable the input field.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set the input field to read-only, default is `false`.
    ///
    /// Unlike [`Self::disabled`], a read-only input keeps the normal appearance
    /// and still can be focused, selected and copied, it only rejects the changes
    /// made by the user.
    pub fn readonly(mut self, readonly: bool) -> Self {
        self.readonly = readonly;
        self
    }

    /// Set the tab index for the input, default is 0.
    pub fn tab_index(mut self, index: isize) -> Self {
        self.tab_index = index;
        self
    }

    /// Sets a custom context menu builder for the input, shown as a native OS menu.
    ///
    /// If set, this overrides the built-in right-click context menu. It shows
    /// only while the state's context menu is enabled, which is the default.
    pub fn context_menu(
        mut self,
        f: impl Fn(NativeMenu, &mut Window, &mut App) -> NativeMenu + 'static,
    ) -> Self {
        self.context_menu_builder = Some(Rc::new(f));
        self
    }

    /// Intercept paste payloads (images, files) before the default text insertion.
    ///
    /// The handler receives the clipboard item and returns whether it took the
    /// paste: `true` stops the `input::Paste` action so the input inserts
    /// nothing, `false` lets it reach the engine, which inserts
    /// `clipboard.text()` as today. Text stays in the `Rope`; images and
    /// copied files (`ClipboardEntry::Image`, `ClipboardEntry::ExternalPaths`)
    /// belong in app-owned state beside the input (e.g. `Attachment`s), never
    /// inside it.
    ///
    /// Known limit: on web `read_from_clipboard()` is `None`, so the handler
    /// is skipped there and the input inserts the plain text itself (a
    /// keyboard paste arrives through the platform input handler, a menu
    /// paste through the asynchronous clipboard read); image paste there
    /// needs `read_from_clipboard_async` and permission, out of scope here.
    pub fn on_paste(
        mut self,
        handler: impl Fn(&gpui::ClipboardItem, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.paste_handler = Some(Rc::new(handler));
        self
    }

    /// The handles and the edit menu of the selection a long press made, unless
    /// the platform draws them; see [`set_native_touch_selection`].
    ///
    /// The menu offers what the native context menu would: Cut, Copy, Paste
    /// and Select All, leaving out what cannot apply right now rather than
    /// disabling it. Cut, Copy and Paste go through the input's actions, so a
    /// custom key binding or an open completion menu sees them the same way.
    fn render_touch_selection(
        state: &TextInputState,
        window: &Window,
        cx: &App,
    ) -> Vec<AnyElement> {
        if is_native_touch_selection(cx) || state.touch_selection(cx).is_none() {
            return Vec::new();
        }
        let capabilities = state.context_menu_capabilities(cx);
        let editable = capabilities.is_editable();
        let copyable = capabilities.is_copyable();
        // Offered whenever the text can change, without peeking at the
        // clipboard: on iOS every read of it shows the system's paste banner,
        // and an empty clipboard pastes nothing.
        let pasteable = editable;
        let selectable = state.text(cx).len() > 0 && !state.is_all_selected(cx);
        let focus_handle = state.presentation(cx).focus_handle().clone();

        let dispatch = {
            move |action: &dyn gpui::Action, window: &mut Window, cx: &mut App| {
                focus_handle.dispatch_action(action, window, cx);
            }
        };
        let mut items = Vec::with_capacity(4);
        if editable && copyable {
            let dispatch = dispatch.clone();
            items.push(EditMenuItem::new(t!("Input.Cut"), move |window, cx| {
                dispatch(&gpui_base::input::Cut, window, cx);
            }));
        }
        if copyable {
            let dispatch = dispatch.clone();
            let state = state.clone();
            items.push(EditMenuItem::new(t!("Input.Copy"), move |window, cx| {
                dispatch(&gpui_base::input::Copy, window, cx);
                state.close_edit_menu(cx);
            }));
        }
        if pasteable {
            let dispatch = dispatch.clone();
            items.push(EditMenuItem::new(t!("Input.Paste"), move |window, cx| {
                dispatch(&gpui_base::input::Paste, window, cx);
            }));
        }
        if selectable {
            let state = state.clone();
            items.push(EditMenuItem::new(
                t!("Input.Select All"),
                move |window, cx| state.select_all_from_edit_menu(window, cx),
            ));
        }

        let drag_state = state.clone();
        let source_state = state.clone();
        TouchSelectionOverlay::new(
            ("input-touch-selection", state.entity_id()),
            move |_, cx| source_state.touch_selection(cx),
        )
        .handles(move |edge, phase, position, _, cx| match phase {
            TouchPhase::Started => drag_state.begin_edge_drag(edge, position, cx),
            TouchPhase::Moved => drag_state.update_edge_drag(position, cx),
            TouchPhase::Ended | TouchPhase::Cancelled => drag_state.end_edge_drag(cx),
        })
        .items(items)
        .into_elements(window, cx)
    }

    fn render_toggle_mask_button(state: &TextInputState, cx: &App) -> impl IntoElement {
        let masked = state.presentation(cx).is_masked();
        Button::new("toggle-mask")
            .icon(if masked {
                IconName::Eye
            } else {
                IconName::EyeOff
            })
            .accessibility_label(if masked {
                t!("Input.Show Password")
            } else {
                t!("Input.Hide Password")
            })
            .xsmall()
            .text()
            .tab_stop(false)
            .on_click({
                let state = state.clone();
                move |_, window, cx| state.toggle_masked(window, cx)
            })
    }

    /// This method must after the refine_style.
    fn render_editor(
        input_state: TextInputState,
        search_panel: Option<AnyElement>,
        _: &Window,
    ) -> impl IntoElement {
        v_flex().size_full().children(search_panel).child(
            div()
                .relative()
                .flex_1()
                .child(input_state.into_any_element()),
        )
    }
}

impl Styled for Input {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// The text alignment of the start edge in the global layout direction.
fn start_text_align(cx: &App) -> TextAlign {
    if LayoutDirection::global(cx).is_rtl() {
        TextAlign::Right
    } else {
        TextAlign::Left
    }
}

impl RenderOnce for Input {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        const LINE_HEIGHT: Rems = Rems(1.25);
        // Unless the caller aligns the text, it starts on the start edge.
        let text_align = self
            .style
            .text
            .text_align
            .unwrap_or_else(|| start_text_align(cx));
        let state = self.state.clone();
        state.install_token_presentation(
            Some(self.token_renderer.unwrap_or_else(|| {
                Rc::new(|token, _, _| super::InputToken::new(token).into_any_element())
            })),
            self.token_click_listener,
            matches!(
                self.content_type,
                Some(InputContentType::Password | InputContentType::NewPassword)
            ),
            cx,
        );
        state.install_token_hover_presentation(self.token_hover_listener, cx);
        // Which kind of input this registers as follows from the state itself.
        sync_focused_input_registry(&state, window, cx);

        state.ensure_highlighter_factory(crate::highlighter::input_highlighter_factory(), cx);
        state.set_editor_style(
            gpui_base::input::InputEditorStyle {
                foreground: cx.theme().foreground,
                muted_foreground: cx.theme().muted_foreground,
                background: cx.theme().editor_background(),
                border: cx.theme().border,
                selection: cx.theme().selection,
                caret: cx.theme().caret,
                diagnostics: gpui_base::input::DiagnosticColors {
                    error: cx.theme().highlight_theme.style.status.error(cx),
                    warning: cx.theme().highlight_theme.style.status.warning(cx),
                    info: cx.theme().highlight_theme.style.status.info(cx),
                    hint: cx.theme().highlight_theme.style.status.hint(cx),
                },
                highlight_styles: cx.theme().highlight_theme.clone(),
                editor_invisible: cx.theme().highlight_theme.style.editor_invisible,
                editor_active_line: cx.theme().highlight_theme.style.editor_active_line,
                editor_gutter_background: cx.theme().highlight_theme.style.editor_gutter_background,
                fold_icon_renderer: Some(Rc::new(|ix, is_folded| {
                    Button::new(("fold-icon", ix))
                        .ghost()
                        .icon(if is_folded {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .xsmall()
                        .rounded(ButtonRounded::Small)
                        .size(px(14.))
                        .selected(is_folded)
                        // Monaco's default `showFoldingControls: "mouseover"`.
                        .when(!is_folded, |button| {
                            button.opacity(0.).hover(|style| style.opacity(1.))
                        })
                        .into_any_element()
                })),
            },
            cx,
        );
        state.set_editor_paddings(
            if state.presentation(cx).is_multi_line() {
                Edges {
                    top: self.size.input_py(),
                    right: self.size.input_px(),
                    bottom: self.size.input_py(),
                    left: if state.presentation(cx).is_code_editor() {
                        self.size.input_px().min(px(6.))
                    } else {
                        self.size.input_px()
                    },
                }
            } else {
                Edges::default()
            },
            cx,
        );
        state.set_disabled(self.disabled, cx);
        state.set_readonly(self.readonly, cx);
        state.set_text_align(text_align, cx);
        let custom = self.context_menu_builder.clone();
        state.on_context_menu(
            Rc::new(move |_, capabilities, position, window, cx| {
                let menu = if let Some(custom) = custom.as_ref() {
                    custom(NativeMenu::new(), window, cx)
                } else {
                    let enabled = !capabilities.is_disabled();
                    // A read-only input can still navigate the code, it only
                    // rejects the items that would change the text.
                    let editable = enabled && !capabilities.is_readonly();
                    let mut menu = NativeMenu::new();
                    if capabilities.is_code_editor() {
                        menu = menu
                            .menu_with_disabled(
                                t!("Input.Go to Definition"),
                                !(enabled && capabilities.has_definition()),
                                Box::new(gpui_base::input::GoToDefinition),
                            )
                            .menu_with_disabled(
                                t!("Input.Show Code Actions"),
                                !(editable && capabilities.has_code_actions()),
                                Box::new(gpui_base::input::ToggleCodeActions),
                            )
                            .separator();
                    }
                    menu.menu_with_disabled(
                        t!("Input.Cut"),
                        !(editable && capabilities.is_copyable()),
                        Box::new(gpui_base::input::Cut),
                    )
                    .menu_with_disabled(
                        t!("Input.Copy"),
                        !capabilities.is_copyable(),
                        Box::new(gpui_base::input::Copy),
                    )
                    // Offered whenever the text can change, without peeking
                    // at the clipboard: the synchronous read is always empty
                    // on the web, and an empty clipboard pastes nothing.
                    .menu_with_disabled(
                        t!("Input.Paste"),
                        !editable,
                        Box::new(gpui_base::input::Paste),
                    )
                    .separator()
                    .menu(
                        t!("Input.Select All"),
                        Box::new(gpui_base::input::SelectAll),
                    )
                };
                menu.show(position, window, cx);
            }),
            cx,
        );
        // The engine ignores `Paste` on a read-only or disabled input; the hook
        // must not see a paste the input itself would refuse.
        let paste_handler = self
            .paste_handler
            .clone()
            .filter(|_| state.presentation(cx).is_editable());
        let mut overlays = state.render_overlays(window, cx);
        overlays
            .floating
            .extend(Self::render_touch_selection(&state, window, cx));

        let presentation = state.presentation(cx);
        let content_type = self.content_type;
        let disabled = self.disabled;
        let is_multi_line = presentation.is_multi_line();
        // Without a content type, a masked input is a password field to screen
        // readers, so they announce it as one.
        let accessibility_role = accessibility_role(
            is_multi_line,
            content_type.or(presentation
                .is_masked()
                .then_some(InputContentType::Password)),
            self.role,
        );
        let input_focused =
            presentation.focus_handle().is_focused(window) && !presentation.is_disabled();
        if input_focused {
            sync_native_content_type(window, content_type, presentation.is_editable());
        }
        let frame_focus_handle = window
            .use_keyed_state(("input-frame-focus", state.entity_id()), cx, |_, cx| {
                cx.focus_handle()
            })
            .read(cx)
            .clone();
        let focused = !presentation.is_disabled()
            && (state.has_selection_focus(window, cx)
                || frame_focus_handle.contains_focused(window, cx));

        let gap_x = match self.size {
            Size::Small => px(4.),
            Size::Large => px(8.),
            _ => px(6.),
        };

        let (bg, _) = input_style(presentation.is_disabled(), cx);
        let bg = if presentation.is_code_editor() {
            cx.theme().editor_background()
        } else {
            bg
        };
        let bg = if presentation.is_disabled() {
            bg.opacity(0.5)
        } else {
            bg
        };
        let prefix = self.prefix;
        let suffix = self.suffix;
        let show_clear_button = self.cleanable
            && presentation.is_editable()
            && !presentation.is_loading()
            && state.text(cx).len() > 0
            && !presentation.is_multi_line();
        let has_suffix =
            suffix.is_some() || presentation.is_loading() || self.mask_toggle || show_clear_button;

        let placeholder = Some(presentation.placeholder().clone()).filter(|p| !p.is_empty());

        // Don't use a mask-derived placeholder ("(___)___-___") as an aria_label fallback.
        let placeholder_is_mask = presentation.mask_placeholder() == placeholder.as_deref();

        let aria_label = match self.aria_label {
            Some(label) => Some(label),
            None if placeholder_is_mask => None,
            None => placeholder.clone(),
        };
        state.set_accessibility(
            gpui_base::input::InputAccessibility::new(accessibility_role)
                .with_label(aria_label)
                .with_placeholder(placeholder)
                .with_accessibility_id(self.accessibility_id)
                .with_value_exposed(exposes_accessibility_value(content_type)),
            cx,
        );
        let id = self
            .id
            .unwrap_or_else(|| ("input", state.entity_id()).into());
        BaseInput::new(id)
            .semantic_owner(state.entity_id())
            .focused(focused)
            .disabled(disabled)
            .track_focus(&frame_focus_handle)
            .on_mouse_down(MouseButton::Right, {
                let state = state.clone();
                move |_, window, cx| {
                    // The text element has already taken focus. Publish its state
                    // before an outer ContextMenu resolves the selection owner;
                    // render-time registration still names the previous input.
                    sync_focused_input_registry(&state, window, cx);
                }
            })
            .when(disabled, |this| {
                this.capture_any_mouse_down(|_, _, cx| cx.stop_propagation())
            })
            .styles(|styles| {
                styles.focused(|style| {
                    style.when(
                        self.appearance && self.bordered && self.focus_bordered,
                        |style| style.border_1().border_color(cx.theme().ring),
                    )
                })
            })
            // The editor owns text semantics. The frame still scopes addon
            // controls, custom IDs and focus styling without registering the
            // editor's FocusHandle a second time.
            .role(RoleOverride::Presentational)
            .flex_row_dir(cx)
            .size_full()
            .line_height(LINE_HEIGHT)
            .when(!is_multi_line, |this| {
                this.input_px(self.size).input_py(self.size)
            })
            .input_h(self.size)
            .input_text_size(self.size)
            .items_center()
            .when(presentation.is_multi_line(), |this| {
                this.h_auto()
                    .when_some(self.height, |this, height| this.h(height))
            })
            .when(self.appearance, |this| {
                this.bg(bg)
                    .rounded(cx.theme().radius)
                    .when(self.bordered, |this| {
                        this.border_1().border_color(cx.theme().input)
                    })
            })
            .items_center()
            .gap(gap_x)
            .refine_style(&self.style)
            .when(
                focused && self.appearance && self.bordered && self.focus_bordered,
                |this| this.focus_ring_style(window, cx),
            )
            .children(prefix.map(|p| {
                div()
                    .when(presentation.is_disabled(), |this| this.opacity(0.5))
                    .child(p)
            }))
            .when(presentation.is_multi_line(), |this| {
                this.child(Self::render_editor(state.clone(), overlays.search, window))
            })
            .when(!presentation.is_multi_line(), |this| {
                this.child(state.clone().into_any_element())
            })
            .when(has_suffix, |this| {
                this.pe(self.size.input_px(), cx).child(
                    h_flex()
                        .flex_row_dir(cx)
                        .id("suffix")
                        .gap(gap_x)
                        .items_center()
                        .cursor_default()
                        .when(presentation.is_disabled(), |this| this.opacity(0.5))
                        .when(presentation.is_loading(), |this| {
                            this.child(Spinner::new().color(cx.theme().muted_foreground))
                        })
                        .when(self.mask_toggle, |this| {
                            this.child(Self::render_toggle_mask_button(&state, cx))
                        })
                        .when(show_clear_button, |this| {
                            this.child(clear_button(cx).on_click({
                                let state = state.clone();
                                move |_, window, cx| {
                                    state.clean(window, cx);
                                    state.focus(window, cx);
                                }
                            }))
                        })
                        .children(suffix),
                )
            })
            .relative()
            .children(overlays.floating)
            .when_some(paste_handler, |this, handler| {
                this.capture_action(move |_: &gpui_base::input::Paste, window, cx| {
                    if let Some(clipboard) = cx.read_from_clipboard() {
                        if handler(&clipboard, window, cx) {
                            cx.stop_propagation();
                        }
                    }
                })
            })
            .render(window, cx)
    }
}
