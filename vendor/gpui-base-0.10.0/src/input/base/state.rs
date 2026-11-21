// RSQL local source changes. See NOTICE for upstream provenance and modifications.
//! A text input field that allows the user to enter text.
//!
//! Based on the `Input` example from the `gpui` crate.
//! https://github.com/zed-industries/zed/blob/main/crates/gpui/examples/input.rs
use gpui::TextAlign;
use gpui::{
    Action, App, AppContext, Bounds, ClipboardItem, Context, Edges, Entity, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, Hsla, InteractiveElement as _, IntoElement, KeyBinding,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Point,
    Render, ScrollEvent, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, UTF16Selection, WeakFocusHandle, Window, actions, div, point,
    prelude::FluentBuilder as _, px,
};
use gpui::{Autocapitalize, TextInputAction, TextInputConfiguration};
use ropey::{Rope, RopeSlice};
use serde::Deserialize;
use std::borrow::Cow;
use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use sum_tree::Bias;
use unicode_segmentation::*;

use super::{
    DiagnosticSet, DisplayMap, FoldPlaceholderRenderer, InputContextMenuCapabilities,
    InputEditorStyle, InputHighlighterFactory, MASK_CHAR, MaskPattern, NativeMenu, NumberStep,
    WrappingIndent,
    blink_cursor::BlinkCursor,
    change::Change,
    cursor::{CursorSelection, Selections},
    element::{
        EditorScrollbar, EditorScrollbarSnapshot, LongestLineKey, OverviewRulerRows, TextElement,
        clamp_horizontal_scroll_offset,
    },
    kind::InputModeKind,
    mask_pattern::normalize_number_input,
    mode::LayoutMode,
    undo_manager::{EditIntent, UndoManager},
};
use crate::actions::{SelectDown, SelectLeft, SelectRight, SelectUp};
use crate::input::blink_cursor::CURSOR_WIDTH;
use crate::input::movement::MoveDirection;
use crate::input::rope_ext::{next_grapheme_boundary, previous_grapheme_boundary};
use crate::input::{
    InputEdit, InputExtras as _, Position, RopeExt as _, element::RIGHT_MARGIN, layout::LastLayout,
};
use crate::overscroll::Overscroll;
use crate::{AutoScroll, StepAction, TestSupportExt as _};

pub(super) const EDITOR_ELEMENT_ID: &str = "input-state";

/// Vertical clearance to retain when revealing a text position.
pub(crate) enum ScrollPadding {
    Minimal,
    SurroundingLines,
}

#[derive(Action, Clone, PartialEq, Eq, Deserialize)]
#[action(namespace = input, no_json)]
pub struct Enter {
    /// Is confirm with secondary.
    pub secondary: bool,
    /// Whether the Shift modifier was held when Enter was pressed.
    pub shift: bool,
}

impl Enter {
    /// Returns true if `action` is a primary `Enter` action (`secondary: false`),
    /// regardless of whether Shift was held.
    pub fn is_primary(action: &dyn Action) -> bool {
        action.partial_eq(&Enter {
            secondary: false,
            shift: false,
        }) || action.partial_eq(&Enter {
            secondary: false,
            shift: true,
        })
    }
}

actions!(
    input,
    [
        ActivateToken,
        Backspace,
        Delete,
        DeleteToBeginningOfLine,
        DeleteToEndOfLine,
        DeleteToPreviousWordStart,
        DeleteToNextWordEnd,
        Indent,
        Outdent,
        IndentInline,
        OutdentInline,
        MoveUp,
        MoveDown,
        MoveLeft,
        MoveRight,
        MoveHome,
        MoveEnd,
        MovePageUp,
        MovePageDown,
        AddCursorAbove,
        AddCursorBelow,
        SelectAll,
        SelectToStartOfLine,
        SelectToEndOfLine,
        SelectToStart,
        SelectToEnd,
        SelectToPreviousWordStart,
        SelectToNextWordEnd,
        ShowCharacterPalette,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        MoveToStartOfLine,
        MoveToEndOfLine,
        MoveToStart,
        MoveToEnd,
        MoveToPreviousWord,
        MoveToNextWord,
        Escape,
        ToggleCodeActions,
        ShowCompletions,
        Search,
        Replace,
        GoToDefinition,
    ]
);

/// An event emitted by an input.
///
/// Subscribe to [`InputEditsEvent`] for the immutable edits associated with a
/// change. `Change` itself remains available for consumers that only need a
/// notification.
#[derive(Clone)]
pub enum InputEvent {
    Change,
    PressEnter { secondary: bool, shift: bool },
    Focus,
    Blur,
}

/// An immutable batch of text edits emitted for one input change.
///
/// Each entry contains the edit and the document immediately after it, in
/// application order. Unlike reading the input state in a `Change` subscriber,
/// this snapshot remains associated with its event when updates are queued.
#[derive(Clone)]
pub struct InputEditsEvent {
    edits: Vec<(InputEdit, Rope)>,
}

impl InputEditsEvent {
    /// The edits and the text after each edit, in application order.
    pub fn edits(&self) -> &[(InputEdit, Rope)] {
        &self.edits
    }
}

pub(super) const CONTEXT: &str = "Input";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some(CONTEXT)),
        KeyBinding::new("shift-backspace", Backspace, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-backspace", Backspace, Some(CONTEXT)),
        KeyBinding::new("delete", Delete, Some(CONTEXT)),
        KeyBinding::new("shift-delete", Delete, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-backspace", DeleteToBeginningOfLine, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-delete", DeleteToEndOfLine, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-backspace", DeleteToPreviousWordStart, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-backspace", DeleteToPreviousWordStart, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-delete", DeleteToNextWordEnd, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-delete", DeleteToNextWordEnd, Some(CONTEXT)),
        KeyBinding::new(
            "enter",
            Enter {
                secondary: false,
                shift: false,
            },
            Some(CONTEXT),
        ),
        KeyBinding::new(
            "shift-enter",
            Enter {
                secondary: false,
                shift: true,
            },
            Some(CONTEXT),
        ),
        KeyBinding::new(
            if cx.platform_info().is_mac() {
                "cmd-enter"
            } else {
                "ctrl-enter"
            },
            Enter {
                secondary: true,
                shift: false,
            },
            Some(CONTEXT),
        ),
        KeyBinding::new("escape", Escape, Some(CONTEXT)),
        KeyBinding::new("up", MoveUp, Some(CONTEXT)),
        KeyBinding::new("down", MoveDown, Some(CONTEXT)),
        KeyBinding::new("left", MoveLeft, Some(CONTEXT)),
        KeyBinding::new("right", MoveRight, Some(CONTEXT)),
        KeyBinding::new("pageup", MovePageUp, Some(CONTEXT)),
        KeyBinding::new("pagedown", MovePageDown, Some(CONTEXT)),
        KeyBinding::new("tab", IndentInline, Some(CONTEXT)),
        KeyBinding::new("shift-tab", OutdentInline, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-]", Indent, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-]", Indent, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-[", Outdent, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-[", Outdent, Some(CONTEXT)),
        KeyBinding::new("shift-left", SelectLeft, Some(CONTEXT)),
        KeyBinding::new("shift-right", SelectRight, Some(CONTEXT)),
        KeyBinding::new("shift-up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("shift-down", SelectDown, Some(CONTEXT)),
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        KeyBinding::new("shift-alt-left", SelectLeft, Some(CONTEXT)),
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        KeyBinding::new("shift-alt-right", SelectRight, Some(CONTEXT)),
        // Avoid Ctrl+Alt+arrows on Linux, where desktops may reserve them.
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-alt-up", AddCursorAbove, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-alt-down", AddCursorBelow, Some(CONTEXT)),
        #[cfg(target_os = "windows")]
        KeyBinding::new("ctrl-alt-up", AddCursorAbove, Some(CONTEXT)),
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        KeyBinding::new("shift-alt-up", AddCursorAbove, Some(CONTEXT)),
        #[cfg(target_os = "windows")]
        KeyBinding::new("ctrl-alt-down", AddCursorBelow, Some(CONTEXT)),
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        KeyBinding::new("shift-alt-down", AddCursorBelow, Some(CONTEXT)),
        KeyBinding::new("home", MoveHome, Some(CONTEXT)),
        KeyBinding::new("end", MoveEnd, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-home", MoveToStart, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-end", MoveToEnd, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-home", SelectToStart, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-end", SelectToEnd, Some(CONTEXT)),
        KeyBinding::new("shift-home", SelectToStartOfLine, Some(CONTEXT)),
        KeyBinding::new("shift-end", SelectToEndOfLine, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-shift-a", SelectToStartOfLine, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-shift-e", SelectToEndOfLine, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("shift-cmd-left", SelectToStartOfLine, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("shift-cmd-right", SelectToEndOfLine, Some(CONTEXT)),
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        KeyBinding::new("alt-shift-left", SelectToPreviousWordStart, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-left", SelectToPreviousWordStart, Some(CONTEXT)),
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        KeyBinding::new("alt-shift-right", SelectToNextWordEnd, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-right", SelectToNextWordEnd, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-a", SelectAll, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-a", SelectAll, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-c", Copy, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-c", Copy, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-x", Cut, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-x", Cut, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-v", Paste, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-v", Paste, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-a", MoveHome, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-left", MoveHome, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-e", MoveEnd, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-right", MoveEnd, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-z", Undo, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-z", Redo, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-up", MoveToStart, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-down", MoveToEnd, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-left", MoveToPreviousWord, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-right", MoveToNextWord, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-left", MoveToPreviousWord, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-right", MoveToNextWord, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-up", SelectToStart, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-down", SelectToEnd, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-z", Undo, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-y", Redo, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-.", ToggleCodeActions, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-.", ToggleCodeActions, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-f", Search, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-f", Search, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-f", Replace, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-h", Replace, Some(CONTEXT)),
    ]);
}

/// A mouse position resolved for a columnar (block) selection.
///
/// A position past the end of a short row has no glyph to land on, so `offset` clips to
/// that row's end and `columns_past_line_end` keeps what the clip dropped. Together they
/// say which column the pointer really reached, which is what a block spanning rows of
/// different lengths has to be measured in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ColumnarPoint {
    offset: usize,
    columns_past_line_end: usize,
}

impl ColumnarPoint {
    pub(super) fn new(offset: usize, columns_past_line_end: usize) -> Self {
        Self {
            offset,
            columns_past_line_end,
        }
    }
}

/// The document and selections a paste was asked to replace, taken when the
/// `Paste` action ran so a clipboard read that resolves later can tell
/// whether it still applies. See [`InputBaseState::paste_target`].
#[derive(Debug, PartialEq)]
struct PasteTarget {
    document_revision: u64,
    selections: Vec<CursorSelection>,
}

/// Caret positioning geometry from the input's latest prepaint.
///
/// Stashed during prepaint so caret-anchored popups can position themselves
/// from the current frame's geometry in their deferred prepaint.
#[derive(Clone, Copy, Debug)]
#[doc(hidden)]
pub struct PrepaintCaretGeometry {
    cursor_bounds: Bounds<Pixels>,
    line_height: Pixels,
    input_bounds: Bounds<Pixels>,
    scroll_offset: Point<Pixels>,
    /// A resize owns visibility while its requested repaint settles.
    pub(super) resize_reveal_owned: bool,
}

impl PrepaintCaretGeometry {
    #[cfg(feature = "test-support")]
    pub fn follows_resize(&self) -> bool {
        self.resize_reveal_owned
    }

    pub(super) fn new(
        cursor_bounds: Bounds<Pixels>,
        line_height: Pixels,
        input_bounds: Bounds<Pixels>,
        scroll_offset: Point<Pixels>,
    ) -> Self {
        Self {
            cursor_bounds,
            line_height,
            input_bounds,
            scroll_offset,
            resize_reveal_owned: false,
        }
    }

    /// The active caret's bounds from the fresh prepaint layout.
    pub fn cursor_bounds(&self) -> Bounds<Pixels> {
        self.cursor_bounds
    }

    /// Line height from the fresh prepaint layout.
    pub fn line_height(&self) -> Pixels {
        self.line_height
    }

    /// The input container bounds from the fresh prepaint.
    pub fn input_bounds(&self) -> Bounds<Pixels> {
        self.input_bounds
    }

    /// The scroll offset that will be in effect after paint, clamped like
    /// `InputBaseState::update_scroll_offset` clamps it.
    pub fn scroll_offset(&self) -> Point<Pixels> {
        self.scroll_offset
    }
}

/// The shared text-editing engine behind [`crate::input::InputState`],
/// [`crate::input::TextareaState`] and [`crate::input::EditorState`].
///
/// `M` is the mode marker: it carries no data and only decides which methods
/// exist, so an ordinary input cannot reach the editor's language features.
///
/// The three states are type aliases of this one, which is why this name is
/// public: an alias is only as usable as the type behind it, so hiding this
/// would leave `InputState` unable to do anything. Prefer naming the aliases
/// — write `InputState`, not `InputBaseState<InputMode>`.
pub struct InputBaseState<M: InputModeKind> {
    /// State only this mode needs. See [`InputModeKind::Extras`].
    pub(crate) extras: M::Extras,
    pub(super) focus_handle: FocusHandle,
    selection_focus: Option<WeakFocusHandle>,
    pub(super) mode: LayoutMode,
    pub(super) text: Rope,
    pub(super) display_map: DisplayMap,
    pub(super) undo_manager: UndoManager,
    pub(super) inline_tokens: Option<Box<super::inline_tokens::InlineTokenStore>>,
    pub(super) pending_token: Option<super::InlineToken>,
    pub(super) replaying_history: bool,
    pub(super) validated_token_edit: bool,
    pub(super) document_revision: u64,
    pub(super) token_presentation: super::InlineTokenPresentation,
    pub(super) token_layout_cache: Option<Box<super::token_presentation::TokenLayoutCache>>,
    /// Real per-frame bounds of the placed token elements, keyed by token
    /// start offset; used for hover payloads and stale-hover reconciliation.
    pub(super) token_bounds: std::collections::HashMap<usize, Bounds<Pixels>>,
    /// The currently hovered token: its entered span, last measured bounds,
    /// and UTF-16 range as of entry. Retained so hover exit can still be
    /// delivered when the token is removed, replaced, scrolled out, or
    /// disabled.
    pub(super) hovered_token: Option<super::token_presentation::HoverSnapshot>,
    pub(super) pending_token_hover_exits: Vec<super::token_presentation::HoverSnapshot>,
    /// The start offset of a pressed token, with the document revision and
    /// pointer position at the press.
    pub(super) pressed_token: Option<(usize, u64, Point<Pixels>)>,
    pub(super) search_session: super::SearchSession,
    /// Advances every time search is explicitly invoked. See
    /// [`InputBaseState::search_activation_revision`].
    pub(super) search_activation_revision: u64,
    pub(super) searchable: bool,
    pub(super) replaceable: bool,
    pub(super) soft_wrap: bool,
    pub(super) wrapping_indent: WrappingIndent,
    pub(super) scroll_beyond_last_line: Option<usize>,
    pub(super) cursor_surrounding_lines: Option<usize>,
    pub(super) blink_cursor: Entity<BlinkCursor>,
    pub(super) loading: bool,
    /// The cursors and selections.
    ///
    /// Always contains at least one selection where index 0 is the active cursor.
    pub(super) selections: Selections,
    /// Range for save the selected word, use to keep word range when drag move.
    pub(super) selected_word_range: Option<CursorSelection>,
    /// The marked range is the temporary insert text on IME typing.
    pub(super) ime_marked_range: Option<CursorSelection>,
    /// Text before an unannounced IME composition, until the next change.
    ime_original_text: Option<Rope>,
    pub(super) last_layout: Option<LastLayout>,
    /// Fresh caret geometry stashed during prepaint for caret-anchored popups.
    ///
    /// Popups are positioned in their deferred prepaint, which runs after the
    /// input's prepaint but before its paint.
    pub(crate) prepaint_caret_geometry: Option<PrepaintCaretGeometry>,
    pub(super) last_cursor: Option<usize>,
    /// Where the platform was last told the caret is, for its IME panel.
    pub(super) ime_anchor: Option<(Point<Pixels>, Option<Bounds<Pixels>>)>,
    /// The input container bounds
    pub(super) input_bounds: Bounds<Pixels>,
    /// The text bounds
    pub(super) last_bounds: Option<Bounds<Pixels>>,
    /// The fully visible part of the window this input last asked its scroll
    /// containers to reveal it in, while focused. `None` while unfocused.
    pub(super) revealed_viewport: Option<Bounds<Pixels>>,
    pub(super) last_selected_range: Option<CursorSelection>,
    /// The current edit batch, copied into an event when a change is emitted.
    pub(super) last_input_edits: Vec<(InputEdit, Rope)>,
    pub(super) selecting: bool,
    /// Anchor point of an in-progress columnar (block) selection.
    pub(super) column_select_start: Option<ColumnarPoint>,
    /// The selection a long press made, with its handles and edit menu.
    pub(super) touch_selection: super::touch::TouchSelection,
    pub(crate) disabled: bool,
    pub(crate) readonly: bool,
    pub(crate) text_align: TextAlign,
    pub(super) masked: bool,
    pub(super) clean_on_escape: bool,
    pub(super) submit_on_enter: bool,
    /// Overrides for [`Self::text_input_configuration`].
    pub(super) input_action: Option<TextInputAction>,
    pub(super) autocorrect: Option<bool>,
    pub(super) autocapitalize: Option<Autocapitalize>,
    pub(super) show_whitespaces: bool,
    /// This flag tells the renderer to prefer the end of the current visual line.
    pub(crate) cursor_line_end_affinity: bool,
    pub(super) pattern: Option<regex::Regex>,
    pub(super) validate: Option<Box<dyn Fn(&str, &mut App) -> bool + 'static>>,
    /// The step strategy for [`super::NumberInput`] to increment/decrement.
    /// See [`Self::step`] and [`Self::step_by`].
    pub(crate) number_step: Option<NumberStep>,
    /// The minimum value for [`super::NumberInput`]. See [`Self::min`].
    pub(crate) number_min: Option<f64>,
    /// The maximum value for [`super::NumberInput`]. See [`Self::max`].
    pub(crate) number_max: Option<f64>,
    pub(crate) scroll_handle: ScrollHandle,
    /// Whether the text stretches past its ends, see [`Self::scroll_bounce`].
    scroll_bounce: bool,
    /// The text's stretch past an end, while it lasts.
    pub(super) overscroll: Overscroll,
    /// The deferred scroll offset to apply on next layout.
    pub(crate) deferred_scroll_offset: Option<Point<Pixels>>,
    /// The size of the scrollable content.
    pub(crate) scroll_size: gpui::Size<Pixels>,
    pub(super) editor_scrollbar_snapshot: Cell<Option<EditorScrollbarSnapshot>>,
    /// Overview ruler marks in display rows, kept between frames.
    pub(super) overview_ruler_rows: std::cell::RefCell<OverviewRulerRows>,
    /// The widest measured unwrapped line width and what it was measured for.
    pub(super) longest_line_width: Cell<Option<(LongestLineKey, Pixels)>>,
    pub(super) editor_paddings: Edges<Pixels>,
    /// The style this state paints with: what was projected onto it, with
    /// every colour left unset resolved from the palette that is current. It
    /// is rebuilt at the top of every render, which is what keeps it current
    /// when the palette changes after the state was built.
    pub(super) paint_override: Option<crate::input::EditorPaint>,
    pub(super) editor_style: InputEditorStyle,
    /// What a consumer projected, kept verbatim so that resolution never
    /// consumes its own output: resolving in place would fill the unset
    /// colours once and then never see them as unset again, which is the same
    /// freeze in a different place.
    projected_editor_style: InputEditorStyle,
    pub(super) fold_placeholder_renderer: Option<FoldPlaceholderRenderer>,
    pub(super) fold_background: Option<Hsla>,
    pub(super) accessibility: Option<InputAccessibility>,
    /// What paints in place of Base's editor scrollbar, when a consumer
    /// supplies its own.
    pub(super) scrollbar_renderer: Option<crate::input::EditorScrollbarRenderer>,

    /// The mask pattern for formatting the input text
    pub(crate) mask_pattern: MaskPattern,
    /// Whether the `mask_pattern` was explicitly set (via [`Self::mask_pattern`]
    /// or [`Self::set_mask_pattern`]), to let [`super::NumberInput`] only apply
    /// its default mask when the user has not made an explicit choice.
    pub(super) mask_pattern_set: bool,
    pub(super) placeholder: SharedString,

    /// Diagnostic currently requested by pointer hover; applications render it.
    pub(super) diagnostic_popover: Option<Rc<crate::input::DiagnosticEntry>>,

    context_menu_handler: Option<
        Rc<dyn Fn(NativeMenu, InputContextMenuCapabilities, Point<Pixels>, &mut Window, &mut App)>,
    >,
    pending_context_menu: Option<(Point<Pixels>, usize)>,
    context_menu_offset: Option<usize>,

    /// Whether the context menu that shows on right-click is enabled.
    ///
    pub(super) enable_context_menu: bool,

    /// A flag to indicate if we are currently inserting a completion item.
    pub(super) completion_inserting: bool,
    pub(super) overlay_action_handler: Option<
        Rc<
            dyn Fn(
                super::InputOverlayKind,
                Box<dyn Action>,
                &mut Window,
                &mut Context<InputBaseState<M>>,
            ) -> bool,
        >,
    >,

    /// A flag to indicate if we have a pending update to the text.
    ///
    /// If true, will call some update (for example LSP, Syntax Highlight) before render.
    _pending_update: bool,
    /// A flag to indicate if we should ignore the next completion event.
    pub(super) silent_replace_text: bool,
    /// A flag to indicate if we should emit InputEvents.
    pub(super) emit_events: bool,

    _subscriptions: Vec<Subscription>,

    pub(super) auto_scroll: AutoScroll,
}

/// Accessibility presentation for the editor's existing keyboard-focus boundary.
///
/// Frames and their addon controls keep their own identities. This projection
/// does not create a focus handle or register another text Tab stop. The engine
/// supplies the live value and guards accessibility actions against its current
/// disabled, read-only and masked state.
#[derive(Clone, Debug)]
pub struct InputAccessibility {
    role: Option<gpui::Role>,
    label: Option<SharedString>,
    placeholder: Option<SharedString>,
    accessibility_id: Option<SharedString>,
    value_exposed: bool,
}

impl InputAccessibility {
    pub fn new(role: Option<gpui::Role>) -> Self {
        Self {
            role,
            label: None,
            placeholder: None,
            accessibility_id: None,
            value_exposed: true,
        }
    }

    pub fn with_label(mut self, label: Option<SharedString>) -> Self {
        self.label = label;
        self
    }

    pub fn with_placeholder(mut self, placeholder: Option<SharedString>) -> Self {
        self.placeholder = placeholder;
        self
    }

    pub fn with_accessibility_id(mut self, id: Option<SharedString>) -> Self {
        self.accessibility_id = id;
        self
    }

    /// Suppress values for secret content even when the mask is revealed.
    pub fn with_value_exposed(mut self, exposed: bool) -> Self {
        self.value_exposed = exposed;
        self
    }

    pub fn role(&self) -> Option<gpui::Role> {
        self.role
    }

    pub fn label(&self) -> Option<&SharedString> {
        self.label.as_ref()
    }

    pub fn placeholder(&self) -> Option<&SharedString> {
        self.placeholder.as_ref()
    }

    pub fn accessibility_id(&self) -> Option<&SharedString> {
        self.accessibility_id.as_ref()
    }

    pub fn value_exposed(&self) -> bool {
        self.value_exposed
    }
}

/// Read-only styling data exposed to presentation facades.
///
/// The fields are private and read through the methods below, so that a new
/// one can be added without breaking the facades.
#[derive(Clone)]
pub struct InputPresentation {
    focus_handle: FocusHandle,
    disabled: bool,
    readonly: bool,
    loading: bool,
    masked: bool,
    multi_line: bool,
    code_editor: bool,
    text_align: TextAlign,
    placeholder: SharedString,
    mask_placeholder: Option<String>,
}

impl InputPresentation {
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn is_readonly(&self) -> bool {
        self.readonly
    }

    /// Returns true if the user is allowed to change the text.
    ///
    /// See also: [`InputBaseState::is_editable`].
    pub fn is_editable(&self) -> bool {
        !self.disabled && !self.readonly
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn is_masked(&self) -> bool {
        self.masked
    }

    pub fn is_multi_line(&self) -> bool {
        self.multi_line
    }

    pub fn is_code_editor(&self) -> bool {
        self.code_editor
    }

    pub fn text_align(&self) -> TextAlign {
        self.text_align
    }

    pub fn placeholder(&self) -> &SharedString {
        &self.placeholder
    }

    /// The placeholder derived from the mask pattern, e.g.: `(___) ___-____`.
    pub fn mask_placeholder(&self) -> Option<&str> {
        self.mask_placeholder.as_deref()
    }
}

impl<M: InputModeKind> EventEmitter<InputEvent> for InputBaseState<M> {}
impl<M: InputModeKind> EventEmitter<InputEditsEvent> for InputBaseState<M> {}

impl<M: InputModeKind> InputBaseState<M> {
    #[doc(hidden)]
    pub fn cursor_layout(&self) -> Option<(Bounds<Pixels>, Pixels)> {
        let layout = self.last_layout.as_ref()?;
        Some((layout.cursor_bounds?, layout.line_height))
    }

    /// Caret geometry from the latest prepaint, for caret-anchored popups.
    ///
    /// Popups positioned in their deferred prepaint read this to anchor to
    /// the current frame's caret instead of the previous frame's paint.
    #[doc(hidden)]
    pub fn prepaint_caret_geometry(&self) -> Option<PrepaintCaretGeometry> {
        self.prepaint_caret_geometry
    }

    pub fn input_bounds(&self) -> Bounds<Pixels> {
        self.input_bounds
    }

    pub fn text_bounds(&self) -> Option<Bounds<Pixels>> {
        self.last_bounds
    }

    /// The cached display map for querying wrapped rows and display positions.
    ///
    /// Reading this does not prepare text or force layout. Before the first
    /// layout, [`DisplayMap::wrap_row_count`] can be zero; prepared empty text
    /// has one row. The count describes content, independent of minimum rows,
    /// auto-grow limits and viewport height.
    ///
    /// Text edits use the cached wrapping parameters. Layout refreshes the
    /// cache after width or font changes; consume that count on a subsequent
    /// render, or use [`Self::prewrap`] to prepare it before the first layout.
    pub fn display_map(&self) -> &DisplayMap {
        &self.display_map
    }

    /// Wrap the text for `font`, `font_size` and `width` before the first
    /// layout, so that [`DisplayMap::wrap_row_count`] is known on the first frame.
    ///
    /// Call it after setting the value and the wrapping indent. `width` is the
    /// width of the input's text area: its width less its padding, and less
    /// the line-number gutter for an editor with line numbers. It wraps as
    /// layout does, a little short of that width and with the state's wrapping
    /// indent. Once the input has been laid out this does nothing, and layout
    /// keeps the wrap up to date. The font and font size must match the input's
    /// actual text style.
    pub fn prewrap(
        &mut self,
        font: gpui::Font,
        font_size: Pixels,
        width: Pixels,
        cx: &mut Context<Self>,
    ) {
        if self.last_layout.is_some() {
            return;
        }
        let wrap_width = (width - RIGHT_MARGIN).max(px(1.));
        self.display_map.set_font(font, font_size, cx);
        self.display_map
            .set_tab_size(self.mode.tab_size().tab_size, cx);
        self.display_map
            .on_layout_changed(self.soft_wrap.then_some(wrap_width), cx);
        self.display_map
            .set_wrapping_indent(self.wrapping_indent, cx);
        self.display_map.ensure_text_prepared(&self.text, cx);
    }

    pub fn diagnostic_popover(&self) -> Option<Rc<crate::input::DiagnosticEntry>> {
        self.diagnostic_popover.clone()
    }

    pub fn presentation(&self) -> InputPresentation {
        InputPresentation {
            focus_handle: self.focus_handle.clone(),
            disabled: self.disabled,
            readonly: self.readonly,
            loading: self.loading,
            masked: self.masked,
            multi_line: self.is_multi_line(),
            code_editor: self.is_code_editor(),
            text_align: self.text_align,
            placeholder: self.placeholder.clone(),
            mask_placeholder: self.mask_pattern.placeholder(),
        }
    }

    /// Whether this input spans more than one line.
    ///
    /// Answered by the mode marker, which is fixed when the state is built.
    /// [`LayoutMode`] holds the row counts and growth policy, not the kind.
    #[inline]
    /// Whether this input paints scrollbars.
    ///
    /// Only a multi-line input can scroll: a single-line input keeps its
    /// caret in view by moving its own offset, and never has a viewport a
    /// user could drag. Adding the editor scrollbar to every input put a
    /// thumb inside every text field, which is a control the field does not
    /// have.
    pub(crate) fn shows_scrollbar(&self) -> bool {
        self.is_multi_line()
    }

    pub fn is_multi_line(&self) -> bool {
        M::MULTI_LINE
    }

    /// Whether this input is a single-line text field. See [`Self::is_multi_line`].
    #[inline]
    pub fn is_single_line(&self) -> bool {
        !M::MULTI_LINE
    }

    /// Whether this input is a source-code editor.
    #[inline]
    pub fn is_code_editor(&self) -> bool {
        M::CODE_EDITOR
    }

    /// Whether the user is allowed to copy the selection out.
    ///
    /// A masked input keeps its value out of the clipboard.
    pub fn is_copyable(&self) -> bool {
        self.selections.iter().any(|sel| !sel.is_empty()) && !self.masked
    }

    pub fn context_menu_capabilities(&self) -> InputContextMenuCapabilities {
        let (go_to_definition, code_actions) = self.extras.context_menu_capabilities();
        InputContextMenuCapabilities::new()
            .disabled(self.disabled)
            .readonly(self.readonly)
            .code_editor(self.is_code_editor())
            .selection(!self.active_selection().is_empty())
            .masked(self.masked)
            .go_to_definition(go_to_definition)
            .code_actions(code_actions)
    }

    pub fn set_text_align(&mut self, text_align: TextAlign, cx: &mut Context<Self>) {
        if !self.is_single_line() || self.text_align == text_align {
            return;
        }

        self.text_align = text_align;
        cx.notify();
    }

    /// Flip the password mask.
    ///
    /// Setting the mask is a single-line method, but flipping it stays here:
    /// the reveal button is rendered from the generic path, and it can only be
    /// switched on through [`crate::input::InputState`] anyway.
    pub fn toggle_masked(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.masked = !self.masked;
        cx.notify();
    }

    pub fn on_context_menu(
        &mut self,
        handler: Rc<
            dyn Fn(NativeMenu, InputContextMenuCapabilities, Point<Pixels>, &mut Window, &mut App),
        >,
    ) {
        self.context_menu_handler = Some(handler);
    }

    /// Keeps the selection highlighted while this popup or one of its children has focus.
    ///
    /// This does not transfer keyboard focus or keep the caret visible. The
    /// association holds only a weak focus handle. Pass `None` to clear it.
    pub fn set_selection_focus(&mut self, focus: Option<FocusHandle>, cx: &mut Context<Self>) {
        self.selection_focus = focus.as_ref().map(FocusHandle::downgrade);
        cx.notify();
    }

    /// Whether the input or its associated selection popup has focus.
    pub fn has_selection_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle.is_focused(window)
            || self
                .selection_focus
                .as_ref()
                .and_then(WeakFocusHandle::upgrade)
                .is_some_and(|focus| focus.is_focused(window) || focus.contains_focused(window, cx))
    }

    /// Build the engine. Each mode's own `new` sets its layout on top of this.
    fn new_in_mode(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle().tab_stop(true);
        let blink_cursor = cx.new(BlinkCursor::new);
        let undo_manager = UndoManager::new();

        let _subscriptions = vec![
            // Key bindings can consume events before on_key_down. Observe input
            // before action dispatch so every keystroke resets the blink delay.
            cx.intercept_keystrokes({
                let focus_handle = focus_handle.clone();
                let blink_cursor = blink_cursor.downgrade();
                move |_, window, cx| {
                    if focus_handle.is_focused(window) {
                        _ = blink_cursor.update(cx, |cursor, cx| cursor.pause(cx));
                    }
                }
            }),
            // Observe the blink cursor to repaint the view when it changes.
            cx.observe(&blink_cursor, |_, _, cx| cx.notify()),
            // Blink the cursor when the window is active, pause when it's not.
            cx.observe_window_activation(window, |input, window, cx| {
                if !window.is_window_active() {
                    input.cancel_mouse_gesture(cx);
                }
                if window.is_window_active() {
                    let focus_handle = input.focus_handle.clone();
                    if focus_handle.is_focused(window) {
                        input.blink_cursor.update(cx, |blink_cursor, cx| {
                            blink_cursor.start(cx);
                        });
                    }
                }
            }),
            cx.observe_window_bounds(window, |input, _, cx| input.cancel_mouse_gesture(cx)),
            cx.on_focus(&focus_handle, window, Self::on_focus),
            cx.on_blur(&focus_handle, window, Self::on_blur),
        ];

        let text_style = window.text_style();

        Self {
            extras: M::Extras::default(),
            focus_handle: focus_handle.clone(),
            selection_focus: None,
            text: "".into(),
            display_map: DisplayMap::new(text_style.font(), window.rem_size(), None),
            search_session: super::SearchSession::default(),
            search_activation_revision: 0,
            searchable: false,
            replaceable: true,
            soft_wrap: true,
            wrapping_indent: WrappingIndent::default(),
            scroll_beyond_last_line: None,
            cursor_surrounding_lines: None,
            blink_cursor,
            undo_manager,
            inline_tokens: None,
            pending_token: None,
            replaying_history: false,
            validated_token_edit: false,
            document_revision: 0,
            token_presentation: Default::default(),
            token_bounds: Default::default(),
            hovered_token: None,
            pending_token_hover_exits: Vec::new(),
            token_layout_cache: None,
            pressed_token: None,
            selections: Selections::default(),
            selected_word_range: None,
            ime_marked_range: None,
            ime_original_text: None,
            input_bounds: Bounds::default(),
            selecting: false,
            touch_selection: Default::default(),
            disabled: false,
            readonly: false,
            text_align: TextAlign::Left,
            masked: false,
            clean_on_escape: false,
            submit_on_enter: false,
            input_action: None,
            autocorrect: None,
            autocapitalize: None,
            show_whitespaces: false,
            loading: false,
            pattern: None,
            validate: None,
            number_step: Some(NumberStep::Fixed(1.)),
            number_min: None,
            number_max: None,
            mode: LayoutMode::default(),
            last_layout: None,
            prepaint_caret_geometry: None,
            last_bounds: None,
            revealed_viewport: None,
            last_selected_range: None,
            last_input_edits: Vec::new(),
            column_select_start: None,
            last_cursor: None,
            ime_anchor: None,
            scroll_handle: ScrollHandle::new(),
            scroll_bounce: crate::ScrollBounce::enabled_by_default(),
            overscroll: Overscroll::default(),
            scroll_size: gpui::size(px(0.), px(0.)),
            editor_scrollbar_snapshot: Cell::new(None),
            overview_ruler_rows: Default::default(),
            longest_line_width: Cell::new(None),
            editor_paddings: Edges::default(),
            deferred_scroll_offset: None,
            placeholder: SharedString::default(),
            mask_pattern: MaskPattern::default(),
            mask_pattern_set: false,
            paint_override: None,
            editor_style: InputEditorStyle::default(),
            projected_editor_style: InputEditorStyle::default(),
            fold_placeholder_renderer: None,
            fold_background: None,
            accessibility: None,
            scrollbar_renderer: None,
            diagnostic_popover: None,
            context_menu_handler: None,
            pending_context_menu: None,
            context_menu_offset: None,
            enable_context_menu: true,
            completion_inserting: false,
            overlay_action_handler: None,
            silent_replace_text: false,
            emit_events: true,
            _subscriptions,
            _pending_update: false,
            cursor_line_end_affinity: false,
            auto_scroll: AutoScroll::default(),
        }
    }

    /// Sets whether the context menu that shows on right-click is enabled.
    ///
    /// The context menu is enabled by default. Disabling it turns off the
    /// right-click menu entirely, including a custom menu set on the element.
    /// A custom menu already replaces the built-in one, so keep this enabled
    /// to show only your own items.
    pub fn context_menu(mut self, enable: bool) -> Self {
        self.enable_context_menu = enable;
        self
    }

    pub fn set_context_menu_enabled(&mut self, enabled: bool) {
        self.enable_context_menu = enabled;
    }

    /// Set whether search UI allows replacement, default is true.
    #[doc(hidden)]
    pub fn replaceable(mut self, allow: bool) -> Self {
        self.replaceable = allow;
        self
    }

    /// Set placeholder
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Set highlighter language for for [`LayoutMode::CodeEditor`] mode.
    pub fn set_highlighter(
        &mut self,
        new_language: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        match &mut self.mode {
            LayoutMode::CodeEditor {
                language,
                highlighter,
                ..
            } => {
                language.set_name(new_language.into());
                *highlighter.borrow_mut() = None;
            }
            _ => {}
        }
        self.refresh(cx);
    }

    fn reset_highlighter(&mut self, cx: &mut Context<Self>) {
        match &mut self.mode {
            LayoutMode::CodeEditor { highlighter, .. } => {
                *highlighter.borrow_mut() = None;
            }
            _ => {}
        }
        cx.notify();
    }

    /// Install the parser/highlighter adapter used by code-editor mode.
    pub fn set_highlighter_factory(
        &mut self,
        factory: InputHighlighterFactory,
        cx: &mut Context<Self>,
    ) {
        self.mode.set_highlighter_factory(factory);
        self._pending_update = true;
        self.token_layout_cache = None;
        cx.notify();
    }

    /// Install a default adapter without replacing an application-provided one.
    pub fn ensure_highlighter_factory(&mut self, factory: InputHighlighterFactory) {
        self.mode.ensure_highlighter_factory(factory);
    }

    pub fn set_editor_style(&mut self, style: InputEditorStyle) {
        self.editor_style = style.clone();
        self.projected_editor_style = style;
    }

    /// Draws a placeholder right after the text of each folded line, given its
    /// buffer row. Clicking it unfolds the line.
    pub fn set_fold_placeholder_renderer(
        &mut self,
        renderer: Option<FoldPlaceholderRenderer>,
        cx: &mut Context<Self>,
    ) {
        self.fold_placeholder_renderer = renderer;
        cx.notify();
    }

    /// Fills each folded line across the editor, as VS Code's
    /// `editor.foldBackground`.
    pub fn set_fold_background(&mut self, color: Option<Hsla>, cx: &mut Context<Self>) {
        self.fold_background = color;
        cx.notify();
    }

    /// Project semantics onto the editor, never onto a second focus owner.
    pub fn set_accessibility(&mut self, accessibility: InputAccessibility) {
        self.accessibility = Some(accessibility);
    }

    fn handle_accessibility_focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.disabled {
            self.focus(window, cx);
        }
    }

    fn handle_accessibility_set_value(
        &mut self,
        data: Option<&gpui::accesskit::ActionData>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(gpui::accesskit::ActionData::Value(value)) = data else {
            return;
        };
        // The action may have been queued before editability changed.
        if self.is_editable() {
            self.replace_all(value.to_string(), window, cx);
        }
    }

    /// Set presentation padding for multi-line text and its scrollbar layout.
    #[doc(hidden)]
    pub fn set_editor_paddings(&mut self, paddings: Edges<Pixels>) {
        self.editor_paddings = paddings;
    }

    /// Supply the element painted in place of the editor's scrollbar, or
    /// `None` to keep Base's. The renderer is called where the editor paints
    /// its scrollbar, with that frame's [`EditorScrollbarContext`]. Like
    /// [`Self::set_editor_style`], this does not notify, so it can be called
    /// while rendering.
    ///
    /// [`EditorScrollbarContext`]: crate::input::EditorScrollbarContext
    pub fn set_scrollbar_renderer(
        &mut self,
        renderer: Option<crate::input::EditorScrollbarRenderer>,
    ) {
        self.scrollbar_renderer = renderer;
    }

    pub fn apply_highlighter_fold_candidates(
        &mut self,
        candidates: Vec<crate::input::FoldRange>,
        cx: &mut Context<Self>,
    ) {
        if self.mode.is_folding() {
            self.display_map.set_fold_candidates(candidates);
        }
        cx.notify();
    }

    #[inline]
    pub fn diagnostics(&self) -> Option<&DiagnosticSet> {
        self.mode.diagnostics()
    }

    #[inline]
    pub fn diagnostics_mut(&mut self) -> Option<&mut DiagnosticSet> {
        self.mode.diagnostics_mut()
    }

    /// Set placeholder
    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    /// Find which line and sub-line the given offset belongs to, along with the position within that sub-line.
    ///
    /// Returns:
    ///
    /// - The index of the line (zero-based) containing the offset.
    /// - The index of the sub-line (zero-based) within the line containing the offset.
    /// - The position of the offset.
    pub(super) fn line_and_position_for_offset(
        &self,
        offset: usize,
    ) -> (usize, usize, Option<Point<Pixels>>) {
        self.line_and_position_for_endpoint(
            offset,
            crate::input::display_map::projection::Endpoint::Caret,
        )
    }

    fn line_and_position_for_endpoint(
        &self,
        offset: usize,
        edge: crate::input::display_map::projection::Endpoint,
    ) -> (usize, usize, Option<Point<Pixels>>) {
        let Some(last_layout) = &self.last_layout else {
            return (0, 0, None);
        };
        let line_height = last_layout.line_height;
        let target_row = self
            .display_map
            .projected_owner(self.text.offset_to_point(offset.min(self.text.len())).row);

        let mut y_offset = last_layout.visible_top;
        for (vi, line) in last_layout.lines.iter().enumerate() {
            let buffer_line = last_layout.visible_buffer_lines[vi];
            if buffer_line == target_row {
                let prev_lines_offset = last_layout.visible_line_byte_offsets[vi];
                let local_offset = offset.saturating_sub(prev_lines_offset).min(line.len());
                let pos = line
                    .position_for_endpoint(
                        local_offset,
                        last_layout,
                        self.editor_paint().is_some()
                            && edge == crate::input::display_map::projection::Endpoint::RangeEnd,
                        edge,
                    )
                    .unwrap_or_else(|| point(px(0.), px(0.)));
                let sub_line_index = (pos.y / line_height) as usize;
                let adjusted_pos = point(pos.x + last_layout.line_number_width, pos.y + y_offset);
                return (vi, sub_line_index, Some(adjusted_pos));
            }

            y_offset += line.size(line_height).height;
        }
        (0, 0, None)
    }

    /// Set the value of the input field: plain text, or [`InputContent`] to
    /// restore text together with its inline tokens. Editing history is
    /// cleared and no [`InputEvent::Change`] is emitted.
    ///
    /// For single-line inputs the caret is placed at the end of the text while
    /// the view is scrolled back to the start, so a long value shows its
    /// beginning instead of its tail (matching HTML `<input>`). Multi-line
    /// inputs reset the selection to `0..0`.
    pub fn set_value(
        &mut self,
        value: impl Into<super::InputContent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let content = value.into();
        self.inline_tokens = None;
        self.undo_manager.set_ignoring(true);
        self.emit_events = false;
        self.replace_text(content.text().clone(), window, cx);
        self.ime_original_text = None;
        self.install_tokens(content);
        self.undo_manager.set_ignoring(false);
        self.emit_events = true;

        self.reset_selection();
        self.reset_lsp_state();
        self.reset_scroll_to_start();

        self.undo_manager.clear();
        cx.notify();
    }

    /// Replace the entire text content while preserving undo history.
    ///
    /// Unlike [`set_value`](Self::set_value), this method records the
    /// replacement in the undo stack, allowing the user to undo/redo
    /// the change. The selection is placed at the end of the new text
    /// for single-line inputs, or cleared (0..0) for multi-line inputs.
    ///
    /// Use this when programmatically replacing the full text but the
    /// user should still be able to undo the operation — e.g. formatting.
    pub fn replace_all(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_text(text, window, cx);
        self.reset_selection();
        self.reset_lsp_state();
        self.reset_scroll_to_start();

        cx.notify();
    }

    /// Perform `f` with the user-facing edit restrictions lifted.
    ///
    /// The `disabled` and `readonly` modes only reject the changes made by the
    /// user, the programmatic APIs must always be able to update the text.
    pub(super) fn with_edits_allowed(&mut self, f: impl FnOnce(&mut Self)) {
        let (was_disabled, was_readonly) = (self.disabled, self.readonly);
        (self.disabled, self.readonly) = (false, false);
        f(self);
        (self.disabled, self.readonly) = (was_disabled, was_readonly);
    }

    /// Insert text at the current cursor position.
    ///
    /// And the cursor will be moved to the end of inserted text.
    pub fn insert(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text: SharedString = text.into();
        self.with_edits_allowed(|this| {
            this.undo_manager.set_pending_intent(EditIntent::Atomic);
            let range_utf16 = this.range_to_utf16(&(this.cursor()..this.cursor()));
            this.replace_text_in_range_silent(Some(range_utf16), &text, window, cx);
            let end = this.active_selection().end;
            this.set_cursor_to(end);
        });
    }

    /// Replace text at the current cursor position.
    ///
    /// And the cursor will be moved to the end of replaced text.
    pub fn replace(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text: SharedString = text.into();
        self.with_edits_allowed(|this| {
            this.undo_manager.set_pending_intent(EditIntent::Atomic);
            this.replace_text_in_range_silent(None, &text, window, cx);
            let end = this.active_selection().end;
            this.set_cursor_to(end);
        });
    }

    fn replace_text(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text: SharedString = text.into();
        self.with_edits_allowed(|this| {
            this.undo_manager.set_pending_intent(EditIntent::Atomic);
            let range = 0..this.text.chars().map(|c| c.len_utf16()).sum();
            this.replace_text_in_range_silent(Some(range), &text, window, cx);
            this.reset_highlighter(cx);
        });
    }

    fn reset_selection(&mut self) {
        self.selections.remove_all_but_active();

        // For single-line inputs the caret is placed at the end of the text
        // (matching HTML `<input>`); multi-line inputs reset the selection to
        // `0..0`.
        if self.is_single_line() {
            let end = self.text.len();
            self.set_cursor_to(end);
        } else {
            self.active_selection_mut().clear();
        }
    }

    fn reset_lsp_state(&mut self) {
        if self.is_code_editor() {
            self._pending_update = true;
            M::reset_language_features(self);
        }
    }

    fn reset_scroll_to_start(&mut self) {
        // Move scroll to the start. For single-line the caret is at the end, so
        // override the cursor-follow scroll for the next painted frame to keep
        // the start visible; the deferred offset is consumed during that paint.
        self.scroll_handle.set_offset(point(px(0.), px(0.)));
        if self.is_single_line() {
            self.deferred_scroll_offset = Some(point(px(0.), px(0.)));
        }
    }

    /// Set with disabled mode.
    ///
    /// See also: [`Self::set_disabled`].
    #[allow(unused)]
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled == disabled {
            return;
        }

        self.disabled = disabled;
        if disabled {
            M::hide_context_menu(self, cx);
            M::clear_inline_completion(self, cx);
        }
        cx.notify();
    }

    /// Set with read-only mode.
    ///
    /// Unlike [`Self::disabled`], a read-only input keeps the normal appearance,
    /// focus, cursor, selection and copy behavior, it only rejects any change
    /// of the text made by the user.
    ///
    /// See also: [`Self::set_readonly`].
    #[allow(unused)]
    pub(crate) fn readonly(mut self, readonly: bool) -> Self {
        self.readonly = readonly;
        self
    }

    pub fn set_readonly(&mut self, readonly: bool, cx: &mut Context<Self>) {
        if self.readonly == readonly {
            return;
        }

        self.readonly = readonly;
        if readonly {
            M::hide_context_menu(self, cx);
            M::clear_inline_completion(self, cx);
            self.search_session.replace_mode = false;
        }
        cx.notify();
    }

    /// Returns true if the user is allowed to change the text.
    ///
    /// This is false when the input is `disabled` or `readonly`, the programmatic
    /// APIs (e.g.: [`Self::set_value`], [`Self::insert`]) are not limited by this.
    pub fn is_editable(&self) -> bool {
        !self.disabled && !self.readonly
    }

    /// Set true to clear the input by pressing Escape key.
    ///
    /// Not on Android, where Escape is also the back button.
    pub fn clean_on_escape(mut self) -> Self {
        self.clean_on_escape = true;
        self
    }

    pub fn set_clean_on_escape(&mut self, clean: bool) {
        self.clean_on_escape = clean;
    }

    /// Set true to treat `Enter` as a submit action in multi-line mode,
    /// while `Shift+Enter` inserts a newline.
    ///
    /// Default is `false` (both `Enter` and `Shift+Enter` insert a newline).
    #[doc(hidden)]
    pub fn submit_on_enter(mut self, submit: bool) -> Self {
        self.submit_on_enter = submit;
        self
    }

    pub fn set_submit_on_enter(&mut self, submit: bool, cx: &mut Context<Self>) {
        self.submit_on_enter = submit;
        cx.notify();
    }

    /// Set the action a software keyboard shows on its enter key, like Search
    /// or Send. Pressing it still emits [`InputEvent::PressEnter`].
    ///
    /// Default is [`TextInputAction::Done`] for a single-line input (and a
    /// multi-line one that submits on enter), and [`TextInputAction::Enter`]
    /// (a line break) for a multi-line input.
    pub fn input_action(mut self, action: TextInputAction) -> Self {
        self.input_action = Some(action);
        self
    }

    /// Set whether the platform may autocorrect the text and offer word
    /// suggestions.
    ///
    /// Default is `false`. Enable this explicitly for fields that accept prose;
    /// masking or revealing text does not change the setting.
    pub fn autocorrect(mut self, autocorrect: bool) -> Self {
        self.autocorrect = Some(autocorrect);
        self
    }

    /// Set how a software keyboard capitalizes the text.
    ///
    /// Default is [`Autocapitalize::None`]. Enable capitalization explicitly for
    /// fields where it is appropriate, independently of autocorrection.
    pub fn autocapitalize(mut self, autocapitalize: Autocapitalize) -> Self {
        self.autocapitalize = Some(autocapitalize);
        self
    }

    /// The text assistance a platform IME (a software keyboard) should give
    /// this input, from [`Self::input_action`], [`Self::autocorrect`] and
    /// [`Self::autocapitalize`] or the defaults they describe.
    pub fn text_input_configuration(&self) -> TextInputConfiguration {
        // Masking is a presentation choice, not a reliable content type: a
        // revealed password must not acquire text assistance. Keep assistance
        // opt-in for structured fields such as passwords, email and URLs.
        let autocorrect = self.autocorrect.unwrap_or(false);
        let input_action =
            self.input_action
                .unwrap_or(if self.is_multi_line() && !self.submit_on_enter {
                    TextInputAction::Enter
                } else {
                    TextInputAction::Done
                });
        TextInputConfiguration {
            autocorrect,
            autocapitalize: self.autocapitalize.unwrap_or(Autocapitalize::None),
            suggestions: autocorrect,
            input_action,
        }
    }

    /// Set whether to show whitespace characters.
    #[doc(hidden)]
    pub fn show_whitespaces(mut self, show: bool) -> Self {
        self.show_whitespaces = show;
        self
    }

    /// Update whether to show whitespace characters.
    pub fn set_show_whitespaces(&mut self, show: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.show_whitespaces = show;
        cx.notify();
    }

    /// Empty rows reserved below the last line of content ("scroll
    /// beyond last line"), code-editor mode only. Mirrors VSCode's
    /// `editor.scrollBeyondLastLine` / Zed's `scroll_beyond_last_line`.
    ///
    /// - `None` (default): half the viewport, floored at
    ///   [`BOTTOM_MARGIN_ROWS`] line-heights.
    /// - `Some(0)`: no trailing space; the cursor sits flush with the
    ///   last row at scroll-max.
    /// - `Some(n)`: exactly `n` rows.
    pub fn scroll_beyond_last_line(mut self, rows: Option<usize>) -> Self {
        self.scroll_beyond_last_line = rows;
        self
    }

    /// Update [`Self::scroll_beyond_last_line`] after construction.
    pub fn set_scroll_beyond_last_line(
        &mut self,
        rows: Option<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.scroll_beyond_last_line == rows {
            return;
        }
        self.scroll_beyond_last_line = rows;
        cx.notify();
    }

    /// Minimum number of lines the cursor is kept clear of the viewport's
    /// top/bottom edge before auto-scroll engages. Mirrors VSCode's
    /// `editor.cursorSurroundingLines` / Zed's `vertical_scroll_margin`.
    /// Orthogonal to [`Self::scroll_beyond_last_line`], which sizes the
    /// empty region; this controls the cursor's resting distance from the
    /// edge.
    ///
    /// - `None` (default): [`BOTTOM_MARGIN_ROWS`] lines, falling back to
    ///   one line on small viewports.
    /// - `Some(n)`: requests `n` lines, clamped to half the viewport.
    ///
    /// In a constrained viewport, edge clearance yields to the cursor's
    /// complete visual row so surrounding space cannot clip the text being edited.
    pub fn cursor_surrounding_lines(mut self, lines: Option<usize>) -> Self {
        self.cursor_surrounding_lines = lines;
        self
    }

    /// Update [`Self::cursor_surrounding_lines`] after construction.
    pub fn set_cursor_surrounding_lines(
        &mut self,
        lines: Option<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cursor_surrounding_lines == lines {
            return;
        }
        self.cursor_surrounding_lines = lines;
        cx.notify();
    }

    /// Set the default value of the input field.
    pub fn default_value(mut self, value: impl Into<SharedString>) -> Self {
        let text: SharedString = value.into();
        self.text = Rope::from(self.normalize_input(&text).as_ref());
        if let Some(diagnostics) = self.mode.diagnostics_mut() {
            diagnostics.reset(&self.text)
        }
        // Note: We can't call display_map.set_text here because it needs cx.
        // The text will be set during prepare_if_need in element.rs
        self._pending_update = true;
        self
    }

    /// Return the value of the input field as an owned string.
    ///
    /// The string is materialized on each call. See [`Self::text`] for the
    /// [`Rope`] the state owns, which is borrowed and costs nothing to read.
    pub fn value(&self) -> SharedString {
        SharedString::new(self.text.to_string())
    }

    /// Return the portion of the value within the input field that
    /// is selected by the user, as an owned string.
    ///
    /// The string is materialized on each call. See [`Self::selected_text`]
    /// for the same selection borrowed out of the [`Rope`] the state owns.
    pub fn selected_value(&self) -> SharedString {
        SharedString::new(self.selected_text().to_string())
    }

    /// Return the value without mask.
    pub fn unmask_value(&self) -> SharedString {
        self.mask_pattern.unmask(&self.text.to_string()).into()
    }

    /// Kept so existing render paths keep compiling.
    ///
    /// Configuration used to be collected by a facade and applied here; the
    /// state now configures itself, so this does nothing and can be deleted at
    /// the call site.
    #[doc(hidden)]
    pub fn prepare(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    /// Return the text [`Rope`] of the input field.
    ///
    /// Borrowed from the state, so reading even a large document copies
    /// nothing. See [`Self::value`] when an owned string is wanted.
    pub fn text(&self) -> &Rope {
        &self.text
    }

    /// Report the change with its own snapshot before the legacy notification.
    fn emit_change(&mut self, cx: &mut Context<Self>) {
        if let Some(original) = self.ime_original_text.take() {
            self.last_input_edits = vec![(
                Self::full_input_edit(&original, &self.text),
                self.text.clone(),
            )];
        }
        cx.emit(InputEditsEvent {
            edits: self.last_input_edits.clone(),
        });
        cx.emit(InputEvent::Change);
    }

    fn full_input_edit(old_text: &Rope, new_text: &Rope) -> InputEdit {
        InputEdit {
            start_byte: 0,
            old_end_byte: old_text.len(),
            new_end_byte: new_text.len(),
            start_position: old_text.offset_to_point(0),
            old_end_position: old_text.offset_to_point(old_text.len()),
            new_end_position: new_text.offset_to_point(new_text.len()),
        }
    }

    /// Return the (0-based) [`Position`] of the cursor.
    pub fn cursor_position(&self) -> Position {
        let offset = self.cursor();
        self.text.offset_to_position(offset)
    }

    /// Set (0-based) [`Position`] of the cursor.
    ///
    /// This will move the cursor to the specified line and column, and update the selection range.
    pub fn set_cursor_position(
        &mut self,
        position: impl Into<Position>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position: Position = position.into();
        let offset = self.text.position_to_offset(&position);

        self.move_to(offset, None, cx);
        self.update_preferred_column();
        self.focus(window, cx);
    }

    /// Focus the input field.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        self.blink_cursor.update(cx, |cursor, cx| {
            cursor.start(cx);
        });
    }

    /// Refresh the input, so the next render re-runs syntax highlighting and
    /// the LSP providers, not just a redraw.
    ///
    /// Assigning the `lsp` providers (or other render-affecting state) at
    /// runtime does not take effect until the text next changes. Call this
    /// afterwards to force the refresh on the next render.
    ///
    /// ```ignore
    /// input.update(cx, |state, cx| {
    ///     state.extras.lsp.hover_provider = Some(provider);
    ///     state.refresh(cx);
    /// });
    /// ```
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self._pending_update = true;
        cx.notify();
    }

    pub(super) fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.undo_manager.break_transaction_coalescing();
        let mirrored = self.arrows_mirrored(cx);
        self.select_all_cursors_to(
            move |s, sel| {
                if mirrored {
                    s.next_boundary(sel.cursor_offset())
                } else {
                    s.previous_boundary(sel.cursor_offset())
                }
            },
            cx,
        );
    }

    pub(super) fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let mirrored = self.arrows_mirrored(cx);
        self.select_all_cursors_to(
            move |s, sel| {
                if mirrored {
                    s.previous_boundary(sel.cursor_offset())
                } else {
                    s.next_boundary(sel.cursor_offset())
                }
            },
            cx,
        );
    }

    pub(super) fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_single_line() {
            return;
        }
        self.select_all_cursors_to_with_affinity(
            |s, sel| {
                s.vertical_selection_target(
                    sel.cursor_offset(),
                    sel.column_anchor,
                    s.line_end_affinity_for(sel),
                    -1,
                )
            },
            true,
            cx,
        );
    }

    pub(super) fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_single_line() {
            return;
        }
        self.select_all_cursors_to_with_affinity(
            |s, sel| {
                s.vertical_selection_target(
                    sel.cursor_offset(),
                    sel.column_anchor,
                    s.line_end_affinity_for(sel),
                    1,
                )
            },
            true,
            cx,
        );
    }

    pub(super) fn on_action_select_all(
        &mut self,
        _: &SelectAll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_all(window, cx);
    }

    pub(super) fn select_to_start(
        &mut self,
        _: &SelectToStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.undo_manager.break_transaction_coalescing();
        self.select_all_cursors_to(|_, _| 0, cx);
    }

    pub(super) fn select_to_end(
        &mut self,
        _: &SelectToEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.undo_manager.break_transaction_coalescing();
        let end = self.text.len();
        self.select_all_cursors_to(move |_, _| end, cx);
    }

    pub(super) fn select_to_start_of_line(
        &mut self,
        _: &SelectToStartOfLine,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.undo_manager.break_transaction_coalescing();
        self.select_all_cursors_to(
            |s, sel| s.start_of_line_at(sel.cursor_offset(), s.line_end_affinity_for(sel)),
            cx,
        );
    }

    pub(super) fn select_to_end_of_line(
        &mut self,
        _: &SelectToEndOfLine,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.undo_manager.break_transaction_coalescing();
        self.select_all_cursors_to_with_affinity(
            |s, sel| {
                (
                    s.end_of_line_at(sel.cursor_offset(), s.line_end_affinity_for(sel)),
                    true,
                )
            },
            false,
            cx,
        );
    }

    pub(super) fn select_to_previous_word(
        &mut self,
        _: &SelectToPreviousWordStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.undo_manager.break_transaction_coalescing();
        let mirrored = self.arrows_mirrored(cx);
        self.select_all_cursors_to(
            move |s, sel| {
                if mirrored {
                    s.next_end_of_word_at(sel.cursor_offset())
                } else {
                    s.previous_start_of_word_at(sel.cursor_offset())
                }
            },
            cx,
        );
    }

    pub(super) fn select_to_next_word(
        &mut self,
        _: &SelectToNextWordEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.undo_manager.break_transaction_coalescing();
        let mirrored = self.arrows_mirrored(cx);
        self.select_all_cursors_to(
            move |s, sel| {
                if mirrored {
                    s.previous_start_of_word_at(sel.cursor_offset())
                } else {
                    s.next_end_of_word_at(sel.cursor_offset())
                }
            },
            cx,
        );
    }

    /// Return the start offset of the previous word.
    /// Return the previous start offset of the word before `offset`.
    pub(super) fn previous_start_of_word_at(&self, offset: usize) -> usize {
        if self.masked {
            // The mask replaces every character, so the displayed text has no
            // word boundaries to move or delete by. Collapse the word to the
            // whole text.
            return 0;
        }

        let offset = self.offset_from_utf16(self.offset_to_utf16(offset));
        // FIXME: Avoid to_string
        let left_part = self.text.slice(0..offset).to_string();

        let target = UnicodeSegmentation::split_word_bound_indices(left_part.as_str())
            .rfind(|(_, s)| !s.trim_start().is_empty())
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.token_boundary(target, Bias::Left)
    }

    /// Return the next end offset of the word after `offset`.
    pub(super) fn next_end_of_word_at(&self, offset: usize) -> usize {
        if self.masked {
            // See `previous_start_of_word_at`.
            return self.text.len();
        }

        let offset = self.offset_from_utf16(self.offset_to_utf16(offset));
        let right_part = self.text.slice(offset..self.text.len()).to_string();

        let target = UnicodeSegmentation::split_word_bound_indices(right_part.as_str())
            .find(|(_, s)| !s.trim_start().is_empty())
            .map(|(i, s)| offset + i + s.len())
            .unwrap_or(self.text.len());
        self.token_boundary(target, Bias::Right)
    }

    /// Get start of line byte offset for the given `offset`.
    ///
    /// When soft wrap is active, first press goes to visual line start,
    /// second press (already at visual start) goes to logical line start.
    pub(super) fn start_of_line_at(&self, offset: usize, line_end_affinity: bool) -> usize {
        if self.is_single_line() {
            return 0;
        }

        let row = self.text.offset_to_point(offset).row;
        let logical_start = self.text.line_start_offset(row);

        if self.soft_wrap && self.is_code_editor() {
            let wrap_point = self
                .display_map
                .offset_to_wrap_display_point_with_affinity(offset, line_end_affinity);
            if let Some(line) = self.display_map.line(row)
                && let Some(range) = line.wrapped_lines.get(wrap_point.local_row)
            {
                let visual_start = logical_start + range.start;
                if offset != visual_start {
                    return visual_start;
                }
            }
        }

        logical_start
    }

    /// Get end of line byte offset for the given `offset`.
    ///
    /// When soft wrap is active, first press goes to visual line end,
    /// second press (already at visual end) goes to logical line end.
    pub(super) fn end_of_line_at(&self, offset: usize, line_end_affinity: bool) -> usize {
        if self.is_single_line() {
            return self.text.len();
        }

        let row = self.text.offset_to_point(offset).row;
        let logical_start = self.text.line_start_offset(row);
        let logical_end = self.text.line_end_offset(row);

        if self.soft_wrap && self.is_code_editor() {
            // Use the row the caret is drawn on: at a wrap boundary the raw offset would name
            // the next row, and a second End press would keep walking down instead of falling
            // through to the logical line end.
            let wrap_point = self
                .display_map
                .offset_to_wrap_display_point_with_affinity(offset, line_end_affinity);
            if let Some(line) = self.display_map.line(row)
                && let Some(range) = line.wrapped_lines.get(wrap_point.local_row)
            {
                let visual_end = logical_start + range.end;
                if offset != visual_end {
                    return visual_end;
                }
            }
        }

        logical_end
    }

    /// Get indent string of next line.
    ///
    /// To get current and next line indent, to return more depth one.
    pub(super) fn indent_of_next_line(&mut self) -> String {
        self.indent_of_next_line_at(self.cursor())
    }

    /// Get indent string of the next line, relative to the given `offset`.
    pub(super) fn indent_of_next_line_at(&mut self, offset: usize) -> String {
        if self.is_single_line() {
            return "".into();
        }

        let mut current_indent = String::new();
        let mut next_indent = String::new();
        let line_end_affinity = self.line_end_affinity_at(offset);
        let current_line_start_pos = self.start_of_line_at(offset, line_end_affinity);
        let next_line_start_pos = self.end_of_line_at(offset, line_end_affinity);
        for c in self.text.slice(current_line_start_pos..).chars() {
            if !c.is_whitespace() {
                break;
            }
            if c == '\n' || c == '\r' {
                break;
            }
            current_indent.push(c);
        }

        for c in self.text.slice(next_line_start_pos..).chars() {
            if !c.is_whitespace() {
                break;
            }
            if c == '\n' || c == '\r' {
                break;
            }
            next_indent.push(c);
        }

        if next_indent.len() > current_indent.len() {
            return next_indent;
        }

        // Smart indent: one extra level after lines ending with a trigger.
        // Skipped inside strings and comments: the trigger is literal text.
        if let Some(rules) = self.mode.language_config() {
            if self.mode.is_smart_indent()
                && M::editing_syntax_context(self, offset) == crate::input::SyntaxContext::Code
            {
                let line_before: String = self
                    .text
                    .slice(current_line_start_pos..offset)
                    .chars()
                    .collect();
                if rules.opens_indent(line_before.trim_end()) {
                    let tab = self.mode.tab_size();
                    if tab.hard_tabs {
                        current_indent.push('\t');
                    } else {
                        for _ in 0..tab.tab_size {
                            current_indent.push(' ');
                        }
                    }
                } else {
                    let line_after = self.text.slice(offset..next_line_start_pos).to_string();
                    if rules.closes_indent(&line_after) {
                        let tab = self.mode.tab_size();
                        if current_indent.ends_with('\t') {
                            current_indent.pop();
                        } else {
                            for _ in 0..tab.tab_size {
                                if !current_indent.ends_with(' ') {
                                    break;
                                }
                                current_indent.pop();
                            }
                        }
                    }
                }
            }
        }

        current_indent
    }

    /// Delete every selection as one batch. Collapsed cursors are first
    /// expanded to a deletion range by `collapsed_target` and non-empty
    /// selections delete their own range.
    ///
    /// `collapsed_intent` is the intent to record when every cursor is
    /// collapsed, which is what makes a run of single-character deletes undo
    /// as one gesture. Deleting a real selection is always atomic.
    fn delete_selections(
        &mut self,
        silent: bool,
        collapsed_intent: EditIntent,
        mut collapsed_target: impl FnMut(&mut Self, usize) -> Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_editable() {
            return;
        }
        let cursors: Vec<CursorSelection> = self.selections.iter().copied().collect();
        let intent = if cursors.iter().all(|sel| sel.is_empty()) {
            collapsed_intent
        } else {
            EditIntent::Atomic
        };
        let mut new_selections: Vec<CursorSelection> = Vec::with_capacity(cursors.len());
        for sel in &cursors {
            let range = if sel.is_empty() {
                collapsed_target(self, sel.cursor_offset())
            } else {
                sel.start..sel.end
            };
            let (start, end) = (range.start.min(range.end), range.start.max(range.end));
            let mut selection = *sel;
            selection.start = start;
            selection.end = end;
            new_selections.push(selection);
        }
        // Capture the user's selections before expanding or merging deletion
        // ranges. The edit ranges cannot reconstruct their original carets.
        self.undo_manager.begin_transaction_with(intent);
        self.undo_manager
            .record_selections(cursors.clone(), cursors.clone());
        self.selections.replace_all(new_selections);
        self.undo_manager.set_pending_intent(intent);

        let was_silent = self.silent_replace_text;
        self.silent_replace_text = silent;
        self.replace_text_in_range(None, "", window, cx);
        self.silent_replace_text = was_silent;
        self.undo_manager
            .record_selections(cursors, self.selections.iter().copied().collect());
        self.undo_manager.commit_transaction();
        self.pause_blink_cursor(cx);
    }

    /// Decide pairing against the pre-edit text, before an opening quote can
    /// change its own syntax context. Ordinary characters never query syntax.
    fn auto_close_target(&self, range: &Range<usize>, text: &str) -> Option<(usize, SharedString)> {
        if self.silent_replace_text
            || self.ime_marked_range.is_some()
            || !self.selections.is_single()
            || !self.active_selection().is_empty()
            || !range.is_empty()
            || range.start != self.cursor()
            || text.chars().count() != 1
            || !self.mode.is_auto_close()
        {
            return None;
        }
        let rules = self.mode.language_config()?;
        if self
            .text
            .chars_at(range.start)
            .next()
            .is_some_and(|c| !c.is_whitespace() && !rules.auto_close_before.contains(c))
        {
            return None;
        }
        for (open, close, not_in) in rules.closing_pairs() {
            let Some(prefix) = open.strip_suffix(text) else {
                continue;
            };
            if !self.text_before_matches(range.start, prefix) {
                continue;
            }
            let start = range.start - prefix.len();
            if open == close
                && (self.is_escaped_at(start)
                    || self
                        .text
                        .chars_at(start)
                        .reversed()
                        .next()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_'))
            {
                continue;
            }
            if not_in.contains(&M::editing_syntax_context(self, start)) {
                continue;
            }
            return Some((open.len(), close.into()));
        }
        None
    }

    fn text_before_matches(&self, offset: usize, text: &str) -> bool {
        offset >= text.len()
            && self.text.is_char_boundary(offset - text.len())
            && self.text.slice(offset - text.len()..offset) == text
    }

    fn text_after_matches(&self, offset: usize, text: &str) -> bool {
        offset + text.len() <= self.text.len()
            && self.text.is_char_boundary(offset + text.len())
            && self.text.slice(offset..offset + text.len()) == text
    }

    fn is_escaped_at(&self, offset: usize) -> bool {
        self.text
            .chars_at(offset)
            .reversed()
            .take_while(|c| *c == '\\')
            .count()
            % 2
            == 1
    }

    /// Cursor target when a typed closer should skip over an existing one.
    ///
    /// Returns `Some(offset)` when `new_text` is a single closer from the
    /// active [`LanguageConfig`](crate::input::language_config::LanguageConfig) that already follows the collapsed cursor and is
    /// not escaped. The caller then moves the cursor without editing text or
    /// history. `None` means insert normally.
    fn skip_over_target(&self, new_text: &str) -> Option<usize> {
        if !self.mode.is_auto_close() || new_text.chars().count() != 1 {
            return None;
        }
        let rules = self.mode.language_config()?;
        let cursor = self.cursor();
        for (open, close, not_in) in rules.closing_pairs() {
            for (index, _) in close.char_indices() {
                if !close[index..].starts_with(new_text)
                    || !self.text_before_matches(cursor, &close[..index])
                    || !self.text_after_matches(cursor, &close[index..])
                {
                    continue;
                }
                let context = M::editing_syntax_context(self, cursor);
                let generated =
                    self.mode
                        .auto_closed_pairs()
                        .contains_closer(cursor, index, close.len());
                if !generated
                    && not_in.contains(&context)
                    && !(context == crate::input::SyntaxContext::String && open == close)
                {
                    continue;
                }
                if self.is_escaped_at(cursor - index) {
                    continue;
                }
                return Some(cursor + new_text.len());
            }
        }
        None
    }

    pub(super) fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        // Nothing to delete at the start of the text. Propagate so an ancestor
        // (e.g. a command palette navigating back a level) can act on it.
        // This is harmless when nothing upstream is bound to backspace. With multiple
        // cursors the others can still delete, so only the lone-cursor case
        // propagates.
        if self.selections.is_single() && self.active_selection().is_empty() && self.cursor() == 0 {
            cx.propagate();
            return;
        }

        // Pair deletion follows automatic closing rules, independently of
        // structural brackets used by Enter.
        if self.mode.is_auto_close()
            && self.selections.is_single()
            && self.active_selection().is_empty()
        {
            let off = self.cursor();
            let deletion = self.mode.language_config().and_then(|rules| {
                rules.closing_pairs().find_map(|(open, close, not_in)| {
                    if !self.text_before_matches(off, open) || !self.text_after_matches(off, close)
                    {
                        return None;
                    }
                    let start = off - open.len();
                    let generated = self
                        .mode
                        .auto_closed_pairs()
                        .contains(start..off, off..off + close.len());
                    if self.is_escaped_at(start)
                        || (!generated && not_in.contains(&M::editing_syntax_context(self, start)))
                    {
                        return None;
                    }
                    Some(start..off + close.len())
                })
            });
            if let Some(range) = deletion {
                let utf16 = self.range_to_utf16(&range);
                self.replace_text_in_range_silent(Some(utf16), "", window, cx);
                return;
            }
        }

        self.delete_selections(
            false,
            EditIntent::Backspace,
            |s, offset| s.previous_boundary(offset)..offset,
            window,
            cx,
        );
    }

    pub(super) fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_selections(
            false,
            EditIntent::DeleteForward,
            |s, offset| offset..s.next_boundary(offset),
            window,
            cx,
        );
    }

    pub(super) fn delete_to_beginning_of_line(
        &mut self,
        _: &DeleteToBeginningOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_selections(
            true,
            EditIntent::Atomic,
            |s, offset| {
                let mut start = s.start_of_line_at(offset, s.line_end_affinity_at(offset));
                if start == offset {
                    start = start.saturating_sub(1);
                }
                start..offset
            },
            window,
            cx,
        );
    }

    pub(super) fn delete_to_end_of_line(
        &mut self,
        _: &DeleteToEndOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_selections(
            true,
            EditIntent::Atomic,
            |s, offset| {
                let mut end = s.end_of_line_at(offset, s.line_end_affinity_at(offset));
                if end == offset {
                    end = (end + 1).clamp(0, s.text.len());
                }
                offset..end
            },
            window,
            cx,
        );
    }

    pub(super) fn delete_previous_word(
        &mut self,
        _: &DeleteToPreviousWordStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_selections(
            true,
            EditIntent::Atomic,
            |s, offset| s.previous_start_of_word_at(offset)..offset,
            window,
            cx,
        );
    }

    pub(super) fn delete_next_word(
        &mut self,
        _: &DeleteToNextWordEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_selections(
            true,
            EditIntent::Atomic,
            |s, offset| offset..s.next_end_of_word_at(offset),
            window,
            cx,
        );
    }

    pub(super) fn enter(&mut self, action: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if M::handle_context_menu_action(self, Box::new(action.clone()), window, cx) {
            return;
        }

        // Clear inline completion on enter (user chose not to accept it)
        if M::has_inline_completion(self) {
            M::clear_inline_completion(self, cx);
        }

        // In multi-line mode with `submit_on_enter` enabled, a plain `Enter`
        // (without Shift) is treated as submit: propagate the action and emit
        // PressEnter without inserting a newline. `Shift+Enter` still inserts
        // a newline. A secondary `Enter` (Cmd/Ctrl+Enter) is always a submit
        // gesture and never inserts a newline.
        let insert_newline =
            self.is_multi_line() && !action.secondary && (!self.submit_on_enter || action.shift);

        if insert_newline {
            if !self.selections.is_single() {
                // Insert a newline (with per-line indent) at every cursor.
                self.selections.merge_overlapping();
                let selections: Vec<CursorSelection> = self.selections.iter().copied().collect();
                let mut edits: Vec<(Range<usize>, String)> = Vec::with_capacity(selections.len());
                for sel in &selections {
                    let indent = if self.is_code_editor() {
                        self.indent_of_next_line_at(sel.cursor_offset())
                    } else {
                        String::new()
                    };
                    edits.push((sel.start..sel.end, format!("\n{}", indent)));
                }
                edits.sort_by_key(|(range, _)| range.start);
                self.replace_text_in_ranges(&edits, window, cx);
                self.pause_blink_cursor(cx);
            } else {
                // Bracket split: Enter between `{|}` produces `{\n  \n}`
                // with the cursor on the middle line. Controlled by smart_indent,
                // independently of automatic closing.
                // Skipped inside strings and comments: splitting there would
                // restructure literal text.
                let mut split = false;
                if self.is_code_editor() && self.active_selection().is_empty() {
                    if let Some(rules) = self.mode.language_config() {
                        if self.mode.is_smart_indent()
                            && M::editing_syntax_context(self, self.cursor())
                                == crate::input::SyntaxContext::Code
                        {
                            let off = self.cursor();
                            let is_pair = rules.brackets.iter().any(|p| {
                                !p.open.is_empty()
                                    && !p.close.is_empty()
                                    && p.open != p.close
                                    && self.text_before_matches(off, p.open.as_ref())
                                    && self.text_after_matches(off, p.close.as_ref())
                            });
                            if is_pair {
                                // Base indent: leading whitespace only.
                                let line_end_affinity = self.line_end_affinity_at(off);
                                let line_start = self.start_of_line_at(off, line_end_affinity);
                                let mut indent = String::new();
                                for c in self.text.slice(line_start..).chars() {
                                    if !c.is_whitespace() || c == '\n' || c == '\r' {
                                        break;
                                    }
                                    indent.push(c);
                                }
                                // Extra level only with smart indent enabled.
                                let mut inner = indent.clone();
                                if self.mode.is_smart_indent() {
                                    let tab = self.mode.tab_size();
                                    if tab.hard_tabs {
                                        inner.push('\t');
                                    } else {
                                        for _ in 0..tab.tab_size {
                                            inner.push(' ');
                                        }
                                    }
                                }
                                let new_line_text = format!("\n{inner}\n{indent}");
                                self.replace_text_in_range_silent(None, &new_line_text, window, cx);
                                self.set_cursor_to(off + 1 + inner.len());
                                self.update_preferred_column();
                                let cursors = self.selections.iter().copied().collect::<Vec<_>>();
                                self.undo_manager
                                    .record_selections(cursors.clone(), cursors);
                                self.pause_blink_cursor(cx);
                                split = true;
                            }
                        }
                    }
                }
                if !split {
                    // Get current line indent
                    let indent = if self.is_code_editor() {
                        self.indent_of_next_line()
                    } else {
                        "".to_string()
                    };

                    // Add newline and indent
                    let new_line_text = format!("\n{}", indent);
                    self.replace_text_in_range_silent(None, &new_line_text, window, cx);
                    self.pause_blink_cursor(cx);
                }
            }
        } else {
            // Single line input or submit-on-enter: just emit the event
            // (e.g.: in a dialog to confirm, or a chat textarea to send).
            self.undo_manager.break_transaction_coalescing();
            cx.propagate();
        }

        cx.emit(InputEvent::PressEnter {
            secondary: action.secondary,
            shift: action.shift,
        });
    }

    pub fn clean(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text("", window, cx);
        self.set_selection(0, 0);
        self.scroll_to(0, None, cx);
    }

    pub(super) fn escape(&mut self, action: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if M::handle_context_menu_action(self, Box::new(action.clone()), window, cx) {
            return;
        }

        // Escape also dismisses a request whose popup has not arrived yet.
        M::hide_context_menu(self, cx);

        // Collapse extra cursors back to the active one first.
        if !self.selections.is_single() {
            self.undo_manager.break_transaction_coalescing();
            self.selections.remove_all_but_active();
            cx.notify();
            return;
        }

        // Clear inline completion on escape
        if M::has_inline_completion(self) {
            M::clear_inline_completion(self, cx);
            return; // Consume the escape, don't propagate
        }
        M::clear_inline_completion(self, cx);

        // The handles and the edit menu are the topmost surface to dismiss.
        if self.touch_selection().is_some() {
            self.dismiss_touch_selection(cx);
            return;
        }

        if self.ime_marked_range.is_some() {
            self.unmark_text(window, cx);
        }

        // Android delivers its back button as escape, and back leaves a field's
        // text alone: it closes what is open or navigates.
        if self.clean_on_escape && !cfg!(target_os = "android") {
            return self.clean(window, cx);
        }

        cx.propagate();
    }

    /// Show the right-click context menu as a native OS menu.
    pub(crate) fn handle_right_click_menu(
        &mut self,
        position: Point<Pixels>,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        if crate::GlobalState::is_in_deferred_context(cx) {
            return;
        }

        self.context_menu_offset = Some(offset);
        if !self.active_selection().contains(offset) {
            self.move_to(offset, None, cx);
        }

        if self.is_code_editor() {
            M::on_hover_definition(self, offset, window, cx);
        }

        if let Some(handler) = self.context_menu_handler.clone() {
            let capabilities = self.context_menu_capabilities();
            let input = cx.weak_entity();
            // The handler runs with the input released, so a custom builder can
            // read it (cursor, selection, text) to build its items.
            window.defer(cx, move |window, cx| {
                if input.upgrade().is_none() {
                    return;
                }
                let previous_focus = window.focused(cx);
                handler(NativeMenu::new(), capabilities, position, window, cx);
                // A drawn menu takes focus synchronously; native menus leave it
                // on the input. Capture it here rather than in the handler.
                input
                    .update(cx, |this, cx| {
                        if window.focused(cx) == previous_focus
                            && !this.focus_handle.is_focused(window)
                            && this.has_selection_focus(window, cx)
                        {
                            // A custom handler can reuse an already-focused popup.
                            return;
                        }
                        let menu_focus = window.focused(cx).filter(|focus| {
                            Some(focus) != previous_focus.as_ref() && *focus != this.focus_handle
                        });
                        this.set_selection_focus(menu_focus, cx);
                    })
                    .ok();
            });
        }
    }

    pub(super) fn add_cursor_above(
        &mut self,
        _: &AddCursorAbove,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_cursor_vertical(-1, cx);
    }

    pub(super) fn add_cursor_below(
        &mut self,
        _: &AddCursorBelow,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_cursor_vertical(1, cx);
    }

    /// Add a new cursor one display line above (`move_lines < 0`) or below each
    /// existing cursor, preserving the column. Cursors that would not move (at
    /// the first/last display row) or that would duplicate an existing cursor
    /// are skipped.
    fn add_cursor_vertical(&mut self, move_lines: isize, cx: &mut Context<Self>) {
        if !self.is_multi_line() {
            return;
        }

        self.pause_blink_cursor(cx);
        // Changing the cursor set ends the editing gesture that came before it.
        self.undo_manager.break_transaction_coalescing();

        let sources: Vec<(usize, Option<(Pixels, usize)>, bool)> = self
            .selections
            .iter()
            .map(|sel| {
                (
                    sel.cursor_offset(),
                    sel.column_anchor,
                    self.line_end_affinity_for(sel),
                )
            })
            .collect();
        let mut offsets: std::collections::HashSet<usize> =
            sources.iter().map(|(offset, _, _)| *offset).collect();

        let mut newest: Option<usize> = None;
        for (offset, anchor, line_end_affinity) in sources {
            let anchor = anchor.or_else(|| self.preferred_column_for(offset));
            let (target, _) = self.vertical_target(offset, anchor, line_end_affinity, move_lines);
            if target == offset || offsets.contains(&target) {
                continue;
            }
            offsets.insert(target);
            let id = self.selections.generate_id();
            let mut cursor = CursorSelection::new(id, target, target);
            cursor.column_anchor = anchor;
            self.selections.add(cursor);
            newest = Some(target);
        }

        if let Some(newest) = newest {
            self.scroll_to(newest, None, cx);
        }
        cx.notify();
    }

    /// Add an additional collapsed cursor at `offset`.
    ///
    /// Rejected when `offset` lands inside an existing selection or exactly on
    /// an existing cursor.
    pub(super) fn add_cursor_at(&mut self, offset: usize, cx: &mut Context<Self>) {
        if !self.is_multi_line() {
            return;
        }

        for sel in self.selections.iter() {
            if sel.contains(offset) {
                return;
            }
            if sel.is_collapsed() && sel.cursor_offset() == offset {
                return;
            }
        }

        self.undo_manager.break_transaction_coalescing();
        let id = self.selections.generate_id();
        let mut selection = CursorSelection::new(id, offset, offset);
        selection.column_anchor = self.preferred_column_for(offset);
        self.selections.add(selection);
        cx.notify();
    }

    /// Build a columnar (block) selection spanning the rows between the two
    /// points, one selection per display row at the same column span.
    ///
    /// The span keeps the columns the pointer sat past the end of a short row, so a row
    /// that runs out of text does not narrow the block for the rows that do reach that
    /// far. Each row is then clipped to its own end, selecting as much of the span as it
    /// actually has.
    pub(super) fn build_columnar_selection(
        &mut self,
        start: ColumnarPoint,
        end: ColumnarPoint,
        cx: &mut Context<Self>,
    ) {
        if !self.is_multi_line() {
            return;
        }

        self.undo_manager.break_transaction_coalescing();

        let (start_row, start_col) = self.columnar_row_column(start);
        let (end_row, end_col) = self.columnar_row_column(end);
        let (start_row, end_row) = (start_row.min(end_row), start_row.max(end_row));
        let (start_col, end_col) = (start_col.min(end_col), start_col.max(end_col));

        let mut new_selections = Vec::with_capacity(end_row - start_row + 1);
        for row in start_row..=end_row {
            let sel_start = self
                .display_map
                .display_row_column_to_offset(row, start_col);
            let sel_end = self.display_map.display_row_column_to_offset(row, end_col);
            let id = self.selections.generate_id();
            let sel_start = self.text.clip_offset(sel_start, Bias::Left);
            let sel_end = self.text.clip_offset(sel_end, Bias::Left);
            new_selections.push(CursorSelection::new(id, sel_start, sel_end));
        }

        self.selections.replace_all(new_selections);
        cx.notify();
    }

    /// Resolve a columnar point to the display row it is on and the column it reaches,
    /// counting the columns beyond the row's end so a short row keeps the full span.
    fn columnar_row_column(&self, point: ColumnarPoint) -> (usize, usize) {
        let display_point = self.display_map.offset_to_wrap_display_point(point.offset);
        let row = self
            .display_map
            .wrap_row_to_display_row(display_point.row)
            .unwrap_or_else(|| {
                self.display_map
                    .nearest_visible_display_row(display_point.row)
            });

        (row, display_point.column + point.columns_past_line_end)
    }

    pub(super) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A token that consumed the press (a double click selecting it) must
        // not also place the caret.
        if window.default_prevented() {
            return;
        }
        // A tap asks for the virtual keyboard. Focus alone does not when it
        // moves from another input, or stays here, after the user hid it.
        if event.button == MouseButton::Left
            && crate::GlobalState::is_touch_press(cx)
            && !self.disabled
            && !self.readonly
        {
            window.request_virtual_keyboard();
        }
        self.undo_manager.break_transaction_coalescing();
        // Treat every press like a cursor move: take focus now rather than
        // after GPUI's focus observer runs, show the caret at once and restart
        // the idle delay, whatever phase the blink was in.
        if !self.disabled {
            self.focus(window, cx);
            self.pause_blink_cursor(cx);
        }
        // Input has its own text selection; suppress the window-level text
        // selection (Root) so it does not start a drag from here.
        crate::global_state::GlobalState::suppress_text_selection(cx);

        // Clear inline completion on any mouse interaction
        M::clear_inline_completion(self, cx);
        // A tap on the touch selection asks for its menu back. Any other press
        // places the caret or starts a new drag, and the handles and the menu
        // no longer belong to what is selected.
        if event.button == MouseButton::Left
            && event.click_count == 1
            && self.reopen_edit_menu_at(event.position, cx)
        {
            return;
        }
        self.dismiss_touch_selection(cx);

        // If there have IME marked range and is empty (Means pressed Esc to abort IME typing)
        // Clear the marked range.
        if let Some(ime_marked_range) = &self.ime_marked_range {
            if ime_marked_range.len() == 0 {
                self.ime_marked_range = None;
            }
        }

        self.selecting = true;
        let (offset, line_end_affinity, columns_past_line_end) =
            self.resolve_mouse_position(event.position);

        if M::on_click(self, event, offset, window, cx) {
            return;
        }

        // Triple click to select line
        if event.button == MouseButton::Left && event.click_count >= 3 {
            self.select_line(offset, window, cx);
            return;
        }

        // Double click to select word
        if event.button == MouseButton::Left && event.click_count == 2 {
            self.select_word(offset, window, cx);
            // A double tap is touch's other way to select a word, and it
            // gets the handles and the menu like a long press does.
            if crate::GlobalState::is_touch_press(cx) {
                self.keep_touch_selection(cx);
            }
            return;
        }

        // Show Mouse context menu
        if event.button == MouseButton::Right {
            if self.enable_context_menu {
                if !self.active_selection().contains(offset) {
                    self.move_to(offset, None, cx);
                }
                self.pending_context_menu = Some((event.position, offset));
            }
            return;
        }

        // Multi-cursor placement, multi-line only.
        if self.is_multi_line() && event.button == MouseButton::Left {
            if event.modifiers.alt
                && (event.modifiers.shift || (cfg!(target_os = "linux") && event.modifiers.control))
            {
                // Alt+Shift starts a block; Linux also accepts Ghostty's Ctrl+Alt.
                // Mark selecting so the drag handler extends the block.
                self.column_select_start = Some(ColumnarPoint::new(offset, columns_past_line_end));
                self.selecting = true;
                self.move_to_with_affinity(offset, None, line_end_affinity, cx);
                return;
            } else if event.modifiers.alt {
                self.add_cursor_at(offset, cx);
                // Keep click-to-add behavior, but use this press as the block
                // anchor if the user continues dragging with the left button.
                self.column_select_start = Some(ColumnarPoint::new(offset, columns_past_line_end));
                return;
            }
        }

        if event.modifiers.shift {
            self.select_to_with_affinity(offset, line_end_affinity, cx);
        } else {
            self.move_to_with_affinity(offset, None, line_end_affinity, cx)
        }
    }

    pub(super) fn on_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button == MouseButton::Right {
            if let Some((position, offset)) = self.pending_context_menu.take() {
                self.handle_right_click_menu(position, offset, window, cx);
            }
        }
        if self.active_selection().is_empty() {
            self.active_selection_mut().reversed = false;
        }
        self.selecting = false;
        self.selected_word_range = None;
        self.column_select_start = None;
        self.auto_scroll.stop();
    }

    pub(super) fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // `last_bounds` is moved by the scroll offset; `input_bounds` is the visible area.
        let within_bounds = self.input_bounds.contains(&event.position);

        if !within_bounds {
            // Clear hover when mouse leaves the input
            M::clear_hover_state(self, cx);
            return;
        }

        // Show diagnostic popover on mouse move
        let (offset, _) = self.index_for_mouse_position(event.position);
        M::on_mouse_move(self, offset, event, window, cx);

        if self.is_code_editor() {
            // Keep the current popover while the pointer stays on the same
            // diagnostic, so moving over a squiggle neither repaints nor
            // rebuilds the popover.
            match self
                .mode
                .diagnostics()
                .and_then(|set| set.for_offset(offset))
            {
                Some(diagnostic) => {
                    if self.diagnostic_popover.as_deref() != Some(diagnostic) {
                        self.diagnostic_popover = Some(Rc::new(diagnostic.clone()));
                        cx.notify();
                    }
                }
                None => {
                    if self.diagnostic_popover.take().is_some() {
                        cx.notify();
                    }
                }
            }
        }
    }

    pub(super) fn on_scroll_wheel(
        &mut self,
        event: &ScrollEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let line_height = self
            .last_layout
            .as_ref()
            .map(|layout| layout.line_height)
            .unwrap_or(window.line_height());
        let delta = event.delta.pixel_delta(line_height);

        let old_offset = self.scroll_handle.offset();
        let precise = matches!(event.delta, gpui::ScrollDelta::Pixels(_));
        if !precise || !self.scroll_bounce || cx.reduce_motion() || !self.overflows_vertically() {
            let displaced = self.overscroll.offset() != 0.;
            self.overscroll.reset();
            if displaced {
                cx.notify();
            }
        }
        let bounces = self.scroll_bounce
            && precise
            && !cx.reduce_motion()
            && delta.y.abs() >= delta.x.abs()
            && self.overflows_vertically();
        if bounces {
            let mut overscroll = std::mem::take(&mut self.overscroll);
            let stretched = overscroll.packet(
                delta.y.as_f32(),
                event.touch_phase,
                self.input_bounds.size.height.as_f32(),
                cx.background_executor().now(),
                |dy| {
                    let before = self.scroll_handle.offset();
                    self.update_scroll_offset(
                        Some(point(before.x + delta.x, before.y + px(dy))),
                        cx,
                    );
                    (self.scroll_handle.offset().y - before.y).as_f32()
                },
            );
            self.overscroll = overscroll;
            if stretched {
                cx.notify();
            }
        } else {
            self.overscroll
                .touch_phase(event.touch_phase, cx.background_executor().now());
            // Axis drift must not drop the end of a vertical drag.
            if matches!(
                event.touch_phase,
                gpui::TouchPhase::Ended | gpui::TouchPhase::Cancelled
            ) && self.overscroll.offset() != 0.
            {
                self.overscroll.packet(
                    0.,
                    event.touch_phase,
                    self.input_bounds.size.height.as_f32(),
                    cx.background_executor().now(),
                    |_| 0.,
                );
                cx.notify();
            }
            self.update_scroll_offset(Some(old_offset + delta), cx);
        }

        // Text that has room to scroll keeps a scroll over it even where the
        // text ends, so reaching the end does not scroll the region around the
        // input; text that fits leaves the scroll to that region.
        if self.scroll_handle.offset() != old_offset || self.overflows_along(delta) {
            cx.stop_propagation();
        }

        if self.diagnostic_popover.take().is_some() {
            cx.notify();
        }
        // The handles follow the text; the menu would sit over whatever
        // scrolls underneath it, so it steps aside until the finger lifts.
        self.edit_menu_on_scroll(event.touch_phase, cx);
    }

    /// Whether the text takes a wheel event moving by `delta` from the
    /// scroll areas around it: it has room to scroll along the axis the
    /// event mostly moves on. Unlike [`Self::overflows_along`], an event that
    /// moves on neither axis, such as a gesture's start or end, goes to text
    /// that can scroll vertically.
    pub(super) fn takes_scroll(&self, delta: Point<Pixels>) -> bool {
        if delta.y.abs() >= delta.x.abs() {
            self.overflows_vertically()
        } else {
            self.scroll_size.width > self.input_bounds.size.width
        }
    }

    /// Whether the text has room to scroll along the axis `delta` mostly
    /// moves on. A trackpad swipe drifts a little off its axis, so the drift
    /// alone does not claim the scroll.
    fn overflows_along(&self, delta: Point<Pixels>) -> bool {
        if delta.y.abs() >= delta.x.abs() {
            delta.y != px(0.) && self.overflows_vertically()
        } else {
            self.scroll_size.width > self.input_bounds.size.width
        }
    }

    /// Whether multi-line text is taller than the input.
    fn overflows_vertically(&self) -> bool {
        self.is_multi_line() && self.scroll_size.height > self.input_bounds.size.height
    }

    pub(super) fn update_scroll_offset(
        &mut self,
        offset: Option<Point<Pixels>>,
        cx: &mut Context<Self>,
    ) {
        let mut offset = offset.unwrap_or(self.scroll_handle.offset());
        let safe_y_range =
            (-self.scroll_size.height + self.input_bounds.size.height).min(px(0.0))..px(0.);

        offset.y = if self.is_single_line() {
            px(0.)
        } else {
            offset.y.clamp(safe_y_range.start, safe_y_range.end)
        };
        offset.x = clamp_horizontal_scroll_offset(
            offset.x,
            self.scroll_size.width,
            self.input_bounds.size.width,
            self.text_align,
        );
        if self.scroll_handle.offset() != offset {
            self.scroll_handle.set_offset(offset);
            cx.notify();
        }
    }

    /// Scroll to make the given offset visible.
    ///
    /// If `direction` is Some, will keep edges at the same side.
    pub(crate) fn scroll_to(
        &mut self,
        offset: usize,
        direction: Option<MoveDirection>,
        cx: &mut Context<Self>,
    ) {
        let padding = if direction.is_some() {
            ScrollPadding::SurroundingLines
        } else {
            ScrollPadding::Minimal
        };
        self.scroll_to_with_padding(offset, direction, padding, cx);
    }

    /// Reveal an offset with independently chosen direction restriction and padding.
    /// Search uses surrounding lines without restricting movement to match order.
    pub(crate) fn scroll_to_with_padding(
        &mut self,
        offset: usize,
        direction: Option<MoveDirection>,
        padding: ScrollPadding,
        cx: &mut Context<Self>,
    ) {
        let Some(last_layout) = self.last_layout.as_ref() else {
            return;
        };
        let Some(bounds) = self.last_bounds.as_ref() else {
            return;
        };

        let mut scroll_offset = self.scroll_handle.offset();
        let was_offset = scroll_offset;
        let line_height = last_layout.line_height;

        let point = self.text.offset_to_point(offset);

        let row = point.row;

        // Resolve the wrapped row even when the target is outside the last layout.
        let display_row = self
            .display_map
            .offset_to_display_row(offset, self.line_end_affinity_at(offset));
        let row_offset_y = line_height * display_row;

        // For Right alignment use 0 margin: the cursor indicator is clamped inside bounds
        // in layout_cursors, so shifting the text here would cause a first-click visual jump.
        let safety_margin = match last_layout.text_align {
            TextAlign::Left => RIGHT_MARGIN,
            TextAlign::Right => px(0.),
            TextAlign::Center => CURSOR_WIDTH,
        };
        if let Some(vi) = last_layout
            .visible_buffer_lines
            .iter()
            .position(|&line| line == row)
        {
            let line = &last_layout.lines[vi];
            let local_offset = offset.saturating_sub(last_layout.visible_line_byte_offsets[vi]);
            if let Some(pos) = line.position_for_index(local_offset, last_layout, false) {
                let bounds_width = bounds.size.width - last_layout.line_number_width;
                let col_offset_x = pos.x;
                if col_offset_x - safety_margin < -scroll_offset.x {
                    // If the position is out of the visible area, scroll to make it visible
                    scroll_offset.x = -col_offset_x + safety_margin;
                } else if col_offset_x + safety_margin > -scroll_offset.x + bounds_width {
                    scroll_offset.x = -(col_offset_x - bounds_width + safety_margin);
                }
            }
        }

        // Scroll the row into view. Use the same edge clearance helper as
        // `TextElement::layout_cursors` so both scroll-into-view paths agree
        // (a mismatch flickered on `Down` at end-of-buffer with a small
        // `cursor_surrounding_lines` override).
        let edge_height =
            if matches!(padding, ScrollPadding::SurroundingLines) && self.is_code_editor() {
                super::element::cursor_surrounding_padding(
                    self.mode.is_auto_grow(),
                    self.cursor_surrounding_lines,
                    super::element::viewport_visible_lines(bounds.size.height, line_height),
                    line_height,
                )
            } else {
                px(0.)
            };
        scroll_offset.y = super::element::reveal_row(
            scroll_offset.y,
            row_offset_y,
            line_height,
            bounds.size.height,
            edge_height,
        );

        // Avoid necessary scroll, when it was already in the correct position.
        if direction == Some(MoveDirection::Up) {
            scroll_offset.y = scroll_offset.y.max(was_offset.y);
        } else if direction == Some(MoveDirection::Down) {
            scroll_offset.y = scroll_offset.y.min(was_offset.y);
        }

        // Clamp the deferred target into the same safe range that
        // `update_scroll_offset` enforces on persist, so paint never shows an
        // over-scrolled frame before the post-paint clamp pulls it back.
        let safe_y_min = (-self.scroll_size.height + self.input_bounds.size.height).min(px(0.));
        scroll_offset.x = scroll_offset.x.min(px(0.));
        scroll_offset.y = scroll_offset.y.clamp(safe_y_min, px(0.));
        self.deferred_scroll_offset = Some(scroll_offset);
        cx.notify();
    }

    pub(super) fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    /// The text of every non-empty selection, in document order.
    fn selected_texts(&self) -> Vec<String> {
        let mut selections: Vec<CursorSelection> = self
            .selections
            .iter()
            .copied()
            .filter(|sel| !sel.is_empty())
            .collect();
        selections.sort_by_key(|sel| sel.start);
        selections
            .iter()
            .map(|sel| self.text.slice(*sel).to_string())
            .collect()
    }

    pub(super) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.selections.iter().all(|sel| sel.is_empty()) {
            // Nothing selected here: let Root copy the window's text selection.
            cx.propagate();
            return;
        }
        if !self.is_copyable() {
            return;
        }

        let texts = self.selected_texts();

        cx.write_to_clipboard(ClipboardItem::new_string(texts.join("\n")));
    }

    pub(super) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_copyable() {
            return;
        }

        let texts = self.selected_texts();

        cx.write_to_clipboard(ClipboardItem::new_string(texts.join("\n")));

        self.undo_manager.set_pending_intent(EditIntent::Atomic);
        self.replace_text_in_range_silent(None, "", window, cx);
    }

    pub(super) fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_editable() {
            return;
        }
        if let Some(clipboard) = cx.read_from_clipboard() {
            self.insert_clipboard(clipboard, window, cx);
            return;
        }
        // The synchronous read is empty on platforms whose clipboard is
        // asynchronous and permission-gated (the web), so fall back to the
        // real read. It has to start here, still inside the user activation
        // that dispatched `Paste`, or the browser refuses the read.
        let read = cx.read_from_clipboard_async();
        let target = self.paste_target();
        cx.spawn_in(window, async move |this, cx| match read.await {
            Ok(Some(clipboard)) => {
                this.update_in(cx, |this, window, cx| {
                    // The read can sit behind a permission prompt for as
                    // long as the user likes. If they edited, moved the
                    // caret or left the input meanwhile, the paste would
                    // land where they no longer mean it to; drop it, as the
                    // browser's own paste event only reaches the focused
                    // input too.
                    if this.is_editable()
                        && this.focus_handle.is_focused(window)
                        && this.paste_target() == target
                    {
                        this.insert_clipboard(clipboard, window, cx);
                    }
                })
                .ok();
            }
            Ok(None) => {}
            Err(error) => tracing::warn!("failed to read the clipboard for paste: {error}"),
        })
        .detach();
    }

    /// Where a paste would go right now: the document as edited so far and
    /// the selections it would replace. Two equal targets mean an edit made
    /// for one still applies to the other.
    fn paste_target(&self) -> PasteTarget {
        PasteTarget {
            document_revision: self.document_revision,
            selections: self.selections.iter().copied().collect(),
        }
    }

    /// Inserts the clipboard text the way `Paste` does: one atomic edit,
    /// newlines dropped on a single-line input, one line per selection when
    /// the counts match on a multi-line one. A clipboard without text (an
    /// image, say) is left alone rather than replacing the selection with
    /// nothing.
    fn insert_clipboard(
        &mut self,
        clipboard: ClipboardItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut new_text) = clipboard.text().filter(|text| !text.is_empty()) else {
            return;
        };
        // A paste is one atomic edit, never part of a typing run.
        self.undo_manager.set_pending_intent(EditIntent::Atomic);

        if !self.is_multi_line() {
            new_text = new_text.replace('\n', "");
            self.replace_text_in_range_silent(None, &new_text, window, cx);
            return;
        }

        // Distribute one clipboard line per selection when the counts match.
        // Otherwise insert the whole clipboard text at each cursor.
        if !self.selections.is_single() {
            self.selections.merge_overlapping();
        }
        let lines: Vec<String> = new_text.split('\n').map(|s| s.to_string()).collect();
        let count = self.selections.len();
        if count > 1 && lines.len() == count {
            let mut selections: Vec<CursorSelection> = self.selections.iter().copied().collect();
            selections.sort_by_key(|sel| sel.start);
            let edits: Vec<(Range<usize>, String)> = selections
                .iter()
                .zip(lines)
                .map(|(sel, line)| (sel.start..sel.end, line))
                .collect();
            self.replace_text_in_ranges(&edits, window, cx);
        } else {
            self.replace_text_in_range_silent(None, &new_text, window, cx);
        }
        self.scroll_to(self.cursor(), None, cx);
    }

    /// The intent of a batch the caller did not label: inserting text at
    /// collapsed cursors is typing, anything else stands on its own.
    fn typing_intent(&self, edits: &[(Range<usize>, String)], new_text: &str) -> EditIntent {
        if !new_text.is_empty()
            && !new_text.contains(['\n', '\r'])
            && edits.iter().all(|(range, _)| range.is_empty())
        {
            EditIntent::Typing
        } else {
            EditIntent::Atomic
        }
    }

    /// Where a cursor stood before an edit made with this intent.
    ///
    /// Backspace and forward delete expand a collapsed cursor over the text
    /// they are about to remove, so the recorded cursor has to collapse back to
    /// the side it came from for undo to restore it where the user left it.
    fn collapse_for_intent(
        intent: EditIntent,
        mut selection: CursorSelection,
        range: &Range<usize>,
    ) -> CursorSelection {
        match intent {
            EditIntent::Backspace => selection.place_at(range.end, None),
            EditIntent::DeleteForward => selection.place_at(range.start, None),
            EditIntent::Typing | EditIntent::Atomic => {}
        }
        selection
    }

    /// Whether an edit rewrites the character before a collapsed cursor, as the
    /// macOS Korean IME does on each keystroke instead of marking text.
    fn rewrites_typed_char(
        &self,
        selection: CursorSelection,
        range: &Range<usize>,
        old_text: &str,
        new_text: &str,
    ) -> bool {
        !self.silent_replace_text
            && selection.is_collapsed()
            && selection.cursor_offset() == range.end
            && old_text.chars().count() == 1
            && !old_text.contains(['\n', '\r'])
            && !new_text.contains(['\n', '\r'])
    }

    fn push_history(
        &mut self,
        text: &Rope,
        range: &Range<usize>,
        new_text: &str,
        requested_intent: Option<EditIntent>,
        selection_before: CursorSelection,
        selection_after: Option<CursorSelection>,
    ) -> bool {
        self.document_revision = self.document_revision.wrapping_add(1);
        let token_delta = self.edit_tokens(range, new_text.len());
        if self.undo_manager.is_ignoring() {
            return false;
        }

        let range =
            text.clip_offset(range.start, Bias::Left)..text.clip_offset(range.end, Bias::Right);
        let old_text = text.slice(range.clone()).to_string();
        let new_range = range.start..range.start + new_text.len();

        let intent = requested_intent.unwrap_or_else(|| {
            if range.is_empty()
                && old_text.is_empty()
                && !new_text.is_empty()
                && !new_text.contains(['\n', '\r'])
                || self.rewrites_typed_char(selection_before, &range, &old_text, new_text)
            {
                EditIntent::Typing
            } else {
                EditIntent::Atomic
            }
        });

        let selection_before = Self::collapse_for_intent(intent, selection_before, &range);
        let selection_after =
            selection_after.unwrap_or_else(|| (new_range.end..new_range.end).into());

        let open_transaction = self.undo_manager.has_open_transaction();
        let mut change = Change::new(range, &old_text, new_range, new_text);
        change.token_delta = token_delta;
        let recorded = self.undo_manager.record_transaction(change, intent);
        // A batch records its own cursor sets. This covers a change that is a
        // transaction on its own.
        if recorded && !open_transaction {
            self.undo_manager
                .record_selections(vec![selection_before], vec![selection_after]);
        }
        recorded
    }

    pub(super) fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        self.replay_history(true, window, cx);
    }

    pub(super) fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        self.replay_history(false, window, cx);
    }

    fn replay_history(&mut self, undo: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_editable() {
            return;
        }
        let replay = if undo {
            self.undo_manager.undo()
        } else {
            self.undo_manager.redo()
        };
        let Some(replay) = replay else {
            return;
        };
        let token_aware = self.inline_tokens.is_some()
            || replay
                .changes
                .iter()
                .any(|change| change.token_delta.is_some());
        let emit_events = self.emit_events;
        if token_aware {
            self.emit_events = false;
            self.last_input_edits.clear();
        }
        self.replaying_history = token_aware;
        self.undo_manager.set_ignoring(true);
        for change in &replay.changes {
            let (range, text) = if undo {
                (change.new_range.into(), &change.old_text)
            } else {
                (change.old_range.into(), &change.new_text)
            };
            let range_utf16 = self.range_to_utf16(&range);
            self.replace_text_in_range_silent(Some(range_utf16), text, window, cx);
            self.replay_tokens(&range, text.len(), change.token_delta.as_deref(), undo);
        }
        self.restore_selections(replay.selections);
        self.mode
            .restore_auto_closed_pairs(replay.auto_closed_pairs.unwrap_or_default());
        if token_aware {
            // Token replay bypasses IME normalization; a stale marked range
            // would otherwise describe text that no longer exists.
            self.ime_marked_range = None;
        }
        self.undo_manager.set_ignoring(false);
        self.replaying_history = false;
        self.emit_events = emit_events;
        if token_aware {
            if emit_events {
                self.emit_change(cx);
            } else {
                self.last_input_edits.clear();
            }
        }
        cx.notify();
    }

    /// Restore a set of selections captured in a transaction, clamping offsets
    /// to the current text length and out of tokens. `None` leaves the current
    /// selections as the replay left them.
    fn restore_selections(&mut self, selections: Option<Vec<CursorSelection>>) {
        let Some(selections) = selections else {
            return;
        };

        let len = self.text.len();
        let restored: Vec<CursorSelection> = selections
            .into_iter()
            .map(|mut sel| {
                let (start, end) = (sel.start.min(len), sel.end.min(len));
                let start_bias = if start == end {
                    Bias::Right
                } else {
                    Bias::Left
                };
                sel.start = self.token_boundary(start, start_bias);
                sel.end = self.token_boundary(end, Bias::Right);
                sel
            })
            .collect();
        self.selections.replace_all(restored);
        self.selections.merge_overlapping();
    }

    /// Get byte offset of the text under the pointer at the right-click that last opened
    /// the context menu.
    ///
    /// A right-click inside the selection keeps the selection and the cursor, so a custom
    /// context menu reads this to find the word under the pointer. The offset is the
    /// UTF-8 offset.
    pub fn context_menu_offset(&self) -> Option<usize> {
        self.context_menu_offset
    }

    /// Get byte offset of the cursor.
    ///
    /// The offset is the UTF-8 offset.
    pub fn cursor(&self) -> usize {
        if let Some(ime_marked_range) = &self.ime_marked_range {
            return ime_marked_range.end;
        }

        self.selections.active().cursor_offset()
    }

    /// Returns the active selection.
    pub(super) fn active_selection(&self) -> &CursorSelection {
        self.selections.active()
    }

    /// Returns a mutable reference to the active selection.
    pub(super) fn active_selection_mut(&mut self) -> &mut CursorSelection {
        self.selections.active_mut()
    }

    /// Sets the active selection to the given range, keeping its `reversed`
    /// and `column_anchor` state untouched.
    pub(super) fn set_selection(&mut self, start: usize, end: usize) {
        let range = self.normalize_token_range(start..end);
        let (start, end) = (range.start, range.end);
        let active = self.active_selection_mut();
        active.start = start;
        active.end = end;
    }

    /// Collapses the active selection to a cursor at the given offset,
    /// clearing `reversed`.
    pub(super) fn set_cursor_to(&mut self, offset: usize) {
        let offset = self.token_boundary(offset, Bias::Right);
        let active = self.active_selection_mut();
        active.start = offset;
        active.end = offset;
        active.reversed = false;
    }

    /// Visible row range in the last laid-out viewport, `None` before first layout.
    pub fn visible_row_range(&self) -> Option<std::ops::Range<usize>> {
        self.last_layout.as_ref().map(|l| l.visible_range.clone())
    }

    /// Current scroll offset of the editor viewport.
    pub fn scroll_offset(&self) -> gpui::Point<gpui::Pixels> {
        self.scroll_handle.offset()
    }

    /// Set scroll offset of the editor viewport.
    ///
    /// The offset will be clamped to the valid range, and applied after the next layout.
    pub fn set_scroll_offset(&mut self, offset: gpui::Point<gpui::Pixels>, cx: &mut Context<Self>) {
        self.deferred_scroll_offset = Some(offset);
        cx.notify();
    }

    /// Laid-out line height; `None` before first layout.
    pub fn line_height(&self) -> Option<gpui::Pixels> {
        self.last_layout.as_ref().map(|l| l.line_height)
    }

    /// Returns the active selection as a byte range into the text.
    ///
    /// With multiple cursors, this reads only the active selection.
    ///
    /// The range is empty (`start == end`) when no text is selected; in
    /// that case the offset equals `cursor()`. Byte offsets are measured
    /// in the underlying rope's byte units.
    pub fn selected_range(&self) -> std::ops::Range<usize> {
        (*self.selections.active()).into()
    }

    /// Returns the secondary selections in their native order as UTF-8 source
    /// byte ranges. The active selection is available through [`Self::selected_range`].
    pub fn secondary_selection_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.selections.iter().skip(1).map(|s| s.start..s.end)
    }

    /// Whether the active selection's caret is at the start of its range.
    ///
    /// Collapsed selections are not reversed. Use this with [`Self::selected_range`]
    /// and [`Self::set_selected_range_with_direction`] to restore a selection.
    pub fn is_selection_reversed(&self) -> bool {
        let selection = self.selections.active();
        !selection.is_empty() && selection.reversed
    }

    pub fn select_all(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.undo_manager.break_transaction_coalescing();
        self.selections.remove_all_but_active();
        self.set_selection(0, self.text.len());
        cx.notify();
    }

    /// Set the selected range using UTF-8 byte offsets, removing additional cursors.
    ///
    /// Non-empty ranges expand to character boundaries. Empty ranges remain empty and are
    /// clipped to the preceding character boundary.
    pub fn set_selected_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        self.set_selected_range_with_direction(range, false, cx);
    }

    /// Set the selected UTF-8 byte range and the active caret direction.
    ///
    /// With `reversed`, the caret is at the start and the anchor is at the end.
    /// Empty ranges collapse to a caret regardless of direction. Range clipping,
    /// token boundaries and removal of additional cursors follow
    /// [`Self::set_selected_range`].
    pub fn set_selected_range_with_direction(
        &mut self,
        range: Range<usize>,
        reversed: bool,
        cx: &mut Context<Self>,
    ) {
        let range = self.normalize_token_range(range);
        let end_bias = if range.start == range.end {
            Bias::Left
        } else {
            Bias::Right
        };
        let start = self.text.clip_offset(range.start, Bias::Left);
        let end = self.text.clip_offset(range.end, end_bias);

        let (anchor, head) = if reversed { (end, start) } else { (start, end) };
        self.move_to(anchor, None, cx);
        self.active_selection_mut().reversed = false;
        self.selected_word_range = None;
        self.select_to(head, cx);
    }

    /// Replace only secondary selections. Reject invalid or intersecting ranges
    /// atomically. Sharing an endpoint is valid for adjacent nonempty ranges.
    pub fn replace_secondary_selections(
        &mut self,
        mut ranges: Vec<Range<usize>>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_multi_line() {
            return false;
        }
        let primary = self.selected_range();
        ranges.sort_by_key(|r| (r.start, r.end));
        let overlaps = |a: &Range<usize>, b: &Range<usize>| {
            a == b
                || a.start < b.end && b.start < a.end
                || a.is_empty() && b.contains(&a.start)
                || b.is_empty() && a.contains(&b.start)
        };
        if ranges.iter().any(|r| {
            r.start > r.end
                || r.end > self.text.len()
                || self.cursor_boundary(r.start, Bias::Left) != r.start
                || self.cursor_boundary(r.end, Bias::Right) != r.end
                || self.normalize_token_range(r.clone()) != *r
                || overlaps(&primary, r)
        }) || ranges.windows(2).any(|p| overlaps(&p[0], &p[1]))
        {
            return false;
        }
        let edit_group = self.active_selection().edit_group;
        if self.selections.len() == ranges.len() + 1
            && self
                .selections
                .iter()
                .skip(1)
                .zip(&ranges)
                .all(|(s, r)| s.edit_group == edit_group && s.start == r.start && s.end == r.end)
        {
            return true;
        }
        #[cfg(feature = "test-support")]
        let primary_before = (*self.active_selection(), self.cursor_line_end_affinity);
        self.selections.remove_all_but_active();
        for range in ranges {
            let id = self.selections.generate_id();
            let mut peer = CursorSelection::new(id, range.start, range.end);
            peer.edit_group = edit_group;
            peer.column_anchor = self.preferred_column_for(range.end);
            self.selections.add(peer);
        }
        #[cfg(feature = "test-support")]
        {
            let after = (*self.active_selection(), self.cursor_line_end_affinity);
            assert_eq!(primary_before, after);
        }
        cx.notify();
        true
    }

    /// Add `range` as an additional selection, keeping the existing ones —
    /// the programmatic equivalent of Alt-clicking another cursor into the
    /// text.
    ///
    /// The range is clipped to the text and expanded to character
    /// boundaries, like [`Self::set_selected_range`]. A range already
    /// covered by a selection adds nothing. Overlapping selections are merged.
    /// On a single-line input there is no second cursor to add, so the call
    /// replaces the selection instead.
    /// The view scrolls to the new selection.
    pub fn add_selection(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        if !self.is_multi_line() {
            self.set_selected_range(range, cx);
            return;
        }

        let range = self.normalize_token_range(range);
        let end_bias = if range.start == range.end {
            Bias::Left
        } else {
            Bias::Right
        };
        let start = self.cursor_boundary(range.start, Bias::Left);
        // Match set_selected_range: expand UTF-8 boundaries first, then place
        // the selection endpoint without splitting a CRLF newline.
        let end = self.cursor_boundary(self.text.clip_offset(range.end, end_bias), Bias::Left);

        for sel in self.selections.iter() {
            if sel.start <= start && end <= sel.end {
                return;
            }
        }

        self.undo_manager.break_transaction_coalescing();
        self.selected_word_range = None;
        self.pause_blink_cursor(cx);
        M::hide_context_menu(self, cx);
        M::clear_inline_completion(self, cx);
        let id = self.selections.generate_id();
        let mut selection = CursorSelection::new(id, start, end);
        selection.column_anchor = self.preferred_column_for(end);
        self.selections.add(selection);
        self.selections.merge_overlapping();
        self.scroll_to(end, None, cx);
        cx.notify();
    }

    /// Resolve a mouse position to a byte offset in the text.
    ///
    /// Also reports the caret's line-end affinity for that offset: `true` when the position
    /// landed on the wrap boundary of a non-final visual row, meaning the caret belongs at the
    /// end of that row rather than at the start of the next one. Callers that place or extend a
    /// selection must pass it on, or clicking past the last glyph of a wrapped row leaves a
    /// caret one row below the pointer.
    pub(crate) fn index_for_mouse_position(&self, position: Point<Pixels>) -> (usize, bool) {
        let (offset, line_end_affinity, _) = self.resolve_mouse_position(position);
        (offset, line_end_affinity)
    }

    /// Resolve a mouse position to a text offset, the caret affinity to use for it, and
    /// how many columns the pointer sits past the end of that row.
    ///
    /// Only a columnar selection needs the third value; everywhere else a position past
    /// the end of a row means the end of that row, and
    /// [`Self::index_for_mouse_position`] is the call to make.
    pub(super) fn resolve_mouse_position(&self, position: Point<Pixels>) -> (usize, bool, usize) {
        // If the text is empty, always return 0
        if self.text.len() == 0 {
            return (0, false, 0);
        }

        let (Some(bounds), Some(last_layout)) =
            (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return (0, false, 0);
        };

        let line_height = last_layout.line_height;
        let line_number_width = last_layout.line_number_width;

        // TIP: About the IBeam cursor
        //
        // If cursor style is IBeam, the mouse mouse position is in the middle of the cursor (This is special in OS)

        // The position is relative to the bounds of the text input
        //
        // bounds.origin:
        //
        // - included the input padding.
        // - included the scroll offset.
        let inner_position = position - bounds.origin - point(line_number_width, px(0.));

        let mut y_offset = last_layout.visible_top;
        // Position relative to the last line walked, kept for a pointer that ends up
        // below every line.
        let mut last_line_pos = None;

        // Traverse visible buffer lines (compact, no hidden entries)
        for (vi, (line_layout, _buffer_line)) in last_layout
            .lines
            .iter()
            .zip(last_layout.visible_buffer_lines.iter())
            .enumerate()
        {
            let line_start_offset = last_layout.visible_line_byte_offsets[vi];

            // Calculate line origin for this display row
            let line_origin = point(px(0.), y_offset);
            let pos = inner_position - line_origin;

            // Return offset by use closest_index_for_x if is single line mode.
            if self.is_single_line() {
                let local_index = line_layout.closest_index_for_x(pos.x, last_layout);
                // A single line never wraps, so there is no boundary to disambiguate.
                return (
                    self.resolve_index(line_start_offset + local_index),
                    false,
                    0,
                );
            }

            // Check if mouse is in this line's bounds
            if let Some((local_index, line_end_affinity)) =
                line_layout.closest_index_for_position(pos, last_layout)
            {
                return (
                    self.resolve_index(line_start_offset + local_index),
                    line_end_affinity,
                    line_layout.columns_past_line_end(pos, last_layout),
                );
            } else if pos.y < px(0.) {
                // Mouse is above this line, return start of this line
                return (self.resolve_index(line_start_offset), false, 0);
            }

            y_offset += line_layout.size(line_height).height;
            last_line_pos = Some(pos);
        }

        // Clamp to the last laid-out row. Returning the end of the whole document
        // would select unseen text before drag autoscroll has reached it.
        let last_position = last_layout
            .lines
            .last()
            .zip(last_line_pos)
            .zip(last_layout.visible_line_byte_offsets.last())
            .map(|((line_layout, pos), line_start)| {
                let last_row_top = (line_layout.size(line_height).height - line_height).max(px(0.));
                let pos = point(pos.x, last_row_top);
                (
                    self.resolve_index(line_start + line_layout.len()),
                    false,
                    line_layout.columns_past_line_end(pos, last_layout),
                )
            });

        last_position.unwrap_or((self.text.len(), false, 0))
    }

    /// Map a display byte index back to a text offset, undoing the mask expansion when the input
    /// is masked.
    fn resolve_index(&self, index: usize) -> usize {
        if self.masked {
            self.text.char_index_to_offset(index / MASK_CHAR.len_utf8())
        } else {
            index.min(self.text.len())
        }
    }

    /// Returns a y offsetted point for the line origin.
    /// Select the text from the current cursor position to the given offset.
    ///
    /// The offset is the UTF-8 offset.
    ///
    /// Ensure the offset use self.next_boundary or self.previous_boundary to get the correct offset.
    /// Extend a single selection so its moving end lands at `offset`, flipping
    /// `reversed` when the ends cross. When a sticky `word_range` is given the
    /// selection is kept covering it.
    fn extend_selection(
        sel: &mut CursorSelection,
        offset: usize,
        word_range: Option<CursorSelection>,
    ) {
        if sel.reversed {
            sel.start = offset;
        } else {
            sel.end = offset;
        }

        if sel.end < sel.start {
            sel.reversed = !sel.reversed;
            std::mem::swap(&mut sel.start, &mut sel.end);
        }

        if let Some(word_range) = word_range {
            if sel.start > word_range.start {
                sel.start = word_range.start;
            }
            if sel.end < word_range.end {
                sel.end = word_range.end;
            }
        }
    }

    /// Extend only the active selection to `offset`. Used by mouse drag.
    pub(crate) fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.select_to_with_affinity(offset, false, cx);
    }

    /// Like [`Self::select_to`], but also carries the caret's line-end affinity.
    ///
    /// See [`Self::move_to_with_affinity`] for why the affinity travels with the offset. Note
    /// that plain [`Self::select_to`] clears the affinity: every offset it is given came from
    /// the text rather than from a visual position, so the caret has no reason to keep sticking
    /// to the end of a wrapped row.
    pub(crate) fn select_to_with_affinity(
        &mut self,
        offset: usize,
        line_end_affinity: bool,
        cx: &mut Context<Self>,
    ) {
        M::clear_inline_completion(self, cx);

        self.cursor_line_end_affinity = line_end_affinity;
        let offset = self.cursor_boundary(offset, Bias::Left);
        let word_range = self.selected_word_range;
        Self::extend_selection(self.active_selection_mut(), offset, word_range);
        let range = self.normalize_token_range(self.selected_range());
        self.set_selection(range.start, range.end);

        if self.active_selection().is_empty() {
            self.update_preferred_column();
        }
        cx.notify()
    }

    /// Extend every selection to the offset produced by `f`, then merge any
    /// selections that now overlap. Used by keyboard selection commands.
    fn select_all_cursors_to(
        &mut self,
        f: impl Fn(&Self, &CursorSelection) -> usize,
        cx: &mut Context<Self>,
    ) {
        self.select_all_cursors_to_with_affinity(|s, sel| (f(s, sel), false), false, cx);
    }

    /// Extend selections with caret affinity, preserving their column anchors even
    /// when vertical movement collapses a selection to a cursor.
    fn select_all_cursors_to_with_affinity(
        &mut self,
        f: impl Fn(&Self, &CursorSelection) -> (usize, bool),
        preserve_column: bool,
        cx: &mut Context<Self>,
    ) {
        self.pause_blink_cursor(cx);
        self.undo_manager.break_transaction_coalescing();
        M::clear_inline_completion(self, cx);

        let mut active_affinity = false;
        let new_selections: Vec<CursorSelection> = self
            .selections
            .iter()
            .map(|sel| {
                let (offset, affinity) = f(self, sel);
                if sel.id == self.active_selection().id {
                    active_affinity = affinity;
                }
                let offset = self.cursor_boundary(offset, Bias::Left);
                let mut new_sel = *sel;
                Self::extend_selection(&mut new_sel, offset, None);
                let range = self.normalize_token_range(new_sel.start..new_sel.end);
                new_sel.start = range.start;
                new_sel.end = range.end;
                if !preserve_column {
                    new_sel.column_anchor =
                        self.preferred_column_for_with_affinity(new_sel.cursor_offset(), affinity);
                } else if new_sel.column_anchor.is_none() {
                    new_sel.column_anchor = self.preferred_column_for_with_affinity(
                        sel.cursor_offset(),
                        self.line_end_affinity_for(sel),
                    );
                }
                new_sel
            })
            .collect();
        self.cursor_line_end_affinity = active_affinity;
        self.selections.replace_all(new_selections);
        self.selections.merge_overlapping();

        self.scroll_to(self.cursor(), None, cx);
        cx.notify()
    }

    /// Unselects the currently selected text.
    pub fn unselect(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.undo_manager.break_transaction_coalescing();
        let offset = self.cursor();
        self.set_cursor_to(offset);
        cx.notify()
    }

    #[inline]
    pub(super) fn offset_from_utf16(&self, offset: usize) -> usize {
        self.text.offset_utf16_to_offset(offset)
    }

    #[inline]
    pub(super) fn offset_to_utf16(&self, offset: usize) -> usize {
        self.text.offset_to_offset_utf16(offset)
    }

    #[inline]
    pub(crate) fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    #[inline]
    pub(super) fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    /// If offset falls on a hidden (folded) line, clamp backward to the end of
    /// the fold header line (last visible position before the fold).
    fn clamp_offset_to_visible_backward(&self, offset: usize) -> usize {
        let line = self.text.offset_to_point(offset).row;
        if self.display_map.is_buffer_line_hidden(line) {
            for fold in self.display_map.folded_ranges() {
                if line > fold.start_line && line <= fold.end_line {
                    return self.text.line_end_offset(fold.start_line);
                }
            }
        }
        offset
    }

    /// If offset falls on a hidden (folded) line, clamp forward to the start of
    /// the fold end line (first visible position after the fold).
    fn clamp_offset_to_visible_forward(&self, offset: usize) -> usize {
        let line = self.text.offset_to_point(offset).row;
        if self.display_map.is_buffer_line_hidden(line) {
            for fold in self.display_map.folded_ranges() {
                if line > fold.start_line && line <= fold.end_line {
                    return self.text.line_start_offset(fold.end_line);
                }
            }
        }
        offset
    }

    /// Clip a cursor/selection offset without splitting a CRLF newline. This does
    /// not alter the rope or its byte/UTF-16 conversion used for exact source APIs.
    pub(super) fn cursor_boundary(&self, offset: usize, bias: Bias) -> usize {
        let offset = self.token_boundary(self.text.clip_offset(offset, bias), bias);
        if offset > 0
            && self.text.char_at(offset - 1) == Some('\r')
            && self.text.char_at(offset) == Some('\n')
        {
            if bias == Bias::Left {
                offset - 1
            } else {
                offset + 1
            }
        } else {
            offset
        }
    }

    pub(super) fn previous_boundary(&self, offset: usize) -> usize {
        let offset =
            self.cursor_boundary(previous_grapheme_boundary(&self.text, offset), Bias::Left);
        self.clamp_offset_to_visible_backward(offset)
    }

    pub(super) fn next_boundary(&self, offset: usize) -> usize {
        let offset = self.cursor_boundary(next_grapheme_boundary(&self.text, offset), Bias::Right);
        self.clamp_offset_to_visible_forward(offset)
    }

    /// Returns the true to let InputElement to render cursor, when Input is focused and current BlinkCursor is visible.
    pub(crate) fn show_cursor(&self, window: &Window, cx: &App) -> bool {
        (self.focus_handle.is_focused(window) || M::is_context_menu_open(self, cx))
            && !self.disabled
            && self.blink_cursor.read(cx).visible()
            && window.is_window_active()
    }

    fn on_focus(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.blink_cursor.update(cx, |cursor, cx| {
            cursor.start(cx);
        });
        cx.emit(InputEvent::Focus);
    }

    fn on_blur(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_mouse_gesture(cx);
        self.revealed_viewport = None;
        if M::is_context_menu_open(self, cx) {
            return;
        }

        M::hide_context_menu(self, cx);
        self.undo_manager.break_transaction_coalescing();

        // NOTE: Do not cancel select, when blur.
        // Because maybe user want to copy the selected text by AppMenuBar (will take focus handle).

        M::clear_hover_state(self, cx);
        self.diagnostic_popover = None;
        M::clear_inline_completion(self, cx);
        self.dismiss_touch_selection(cx);
        self.blink_cursor.update(cx, |cursor, cx| {
            cursor.stop(cx);
        });
        self.clamp_number_value(window, cx);
        cx.emit(InputEvent::Blur);
        cx.notify();
    }

    /// Clamp the number value to the `min`/`max` range, used on blur.
    ///
    /// Out-of-range values are allowed while typing (e.g. `1` is an
    /// intermediate state of `15` when min is 10), and clamped on blur.
    fn clamp_number_value(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_single_line() {
            return;
        }
        if !matches!(self.mask_pattern, MaskPattern::Number { .. }) {
            return;
        }
        if self.number_min.is_none() && self.number_max.is_none() {
            return;
        }

        let Ok(value) = self.unmask_value().parse::<f64>() else {
            return;
        };

        let clamped = match (self.number_min, self.number_max) {
            (Some(min), _) if value < min => min,
            (_, Some(max)) if value > max => max,
            _ => return,
        };

        // The clamped value must pass the `pattern`/`validate` check,
        // otherwise keep the value as is.
        let new_text = clamped.to_string();
        if !self.is_valid_input(&new_text, cx) {
            return;
        }

        let range = self.range_to_utf16(&(0..self.text.len()));
        self.replace_text_in_range_silent(Some(range), &new_text, window, cx);
    }

    pub(super) fn pause_blink_cursor(&mut self, cx: &mut Context<Self>) {
        self.blink_cursor.update(cx, |cursor, cx| {
            cursor.pause(cx);
        });
    }

    pub(super) fn cancel_mouse_gesture(&mut self, cx: &mut Context<Self>) {
        let was_dragging = self.selecting || self.column_select_start.is_some();
        self.selecting = false;
        self.column_select_start = None;
        self.auto_scroll.stop();
        if was_dragging {
            cx.notify();
        }
    }

    pub(super) fn on_drag_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.cancel_mouse_gesture(cx);
            return;
        }
        if self.text.len() == 0 {
            return;
        }

        if self.last_layout.is_none() {
            return;
        }

        if !self.focus_handle.is_focused(window) {
            return;
        }

        if !self.selecting {
            return;
        }

        self.auto_scroll.last_drag_position = Some(event.position);
        let (offset, line_end_affinity, columns_past_line_end) =
            self.resolve_mouse_position(event.position);
        if let Some(start) = self.column_select_start {
            let end = ColumnarPoint::new(offset, columns_past_line_end);
            self.build_columnar_selection(start, end, cx);
        } else {
            self.select_to_with_affinity(offset, line_end_affinity, cx);
        }

        if !self.is_single_line() {
            let delta = AutoScroll::compute_delta(event.position.y, self.input_bounds);
            // Input's ScrollHandle uses negative-y-is-down; negate the positive-towards-bottom delta.
            let scroll_delta = delta.map(|d| -d);
            self.auto_scroll.set(scroll_delta, cx, |delta, state, cx| {
                let current = state.scroll_handle.offset();
                state.update_scroll_offset(Some(point(current.x, current.y + delta)), cx);
                if let Some(pos) = state.auto_scroll.last_drag_position {
                    let (offset, line_end_affinity, columns_past_line_end) =
                        state.resolve_mouse_position(pos);
                    if let Some(start) = state.column_select_start {
                        let end = ColumnarPoint::new(offset, columns_past_line_end);
                        state.build_columnar_selection(start, end, cx);
                    } else {
                        state.select_to_with_affinity(offset, line_end_affinity, cx);
                    }
                }
            });
        }
    }

    /// Normalize the inserted text before applying it to the input.
    ///
    /// For number inputs (with [`MaskPattern::Number`]), this converts
    /// full-width number characters into their ASCII equivalents,
    /// e.g. `12。5` -> `12.5`.
    pub(super) fn normalize_input<'a>(&self, new_text: &'a str) -> Cow<'a, str> {
        let normalized = if matches!(self.mask_pattern, MaskPattern::Number { .. }) {
            normalize_number_input(new_text)
        } else {
            Cow::Borrowed(new_text)
        };

        if self.is_single_line() && normalized.contains(['\n', '\r']) {
            Cow::Owned(normalized.replace(['\n', '\r'], ""))
        } else {
            normalized
        }
    }

    pub(crate) fn is_valid_input(&self, new_text: &str, cx: &mut Context<Self>) -> bool {
        if new_text.is_empty() {
            return true;
        }

        if let Some(validate) = &self.validate {
            if !validate(new_text, cx) {
                return false;
            }
        }

        if !self.mask_pattern.is_valid(new_text) {
            return false;
        }

        let Some(pattern) = &self.pattern else {
            return true;
        };

        pattern.is_match(new_text)
    }

    /// Set the mask pattern for formatting the input text.
    ///
    /// The pattern can contain:
    /// - 9: Any digit or dot
    /// - A: Any letter
    /// - *: Any character
    /// - Other characters will be treated as literal mask characters
    ///
    /// Example: "(999)999-999" for phone numbers
    pub fn mask_pattern(mut self, pattern: impl Into<MaskPattern>) -> Self {
        self.mask_pattern = pattern.into();
        self.mask_pattern_set = true;
        if let Some(placeholder) = self.mask_pattern.placeholder() {
            self.placeholder = placeholder.into();
        }
        self
    }

    pub fn set_mask_pattern(
        &mut self,
        pattern: impl Into<MaskPattern>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mask_pattern = pattern.into();
        self.mask_pattern_set = true;
        if let Some(placeholder) = self.mask_pattern.placeholder() {
            self.placeholder = placeholder.into();
        }
        cx.notify();
    }

    /// Apply the default numeric mask unless the caller explicitly selected a mask.
    pub fn ensure_number_mask(&mut self) {
        if self.mask_pattern_set {
            return;
        }
        self.mask_pattern = MaskPattern::Number {
            separator: None,
            fraction: None,
        };
    }

    pub(super) fn set_input_bounds(&mut self, new_bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        let wrap_width_changed = self.input_bounds.size.width != new_bounds.size.width;
        self.input_bounds = new_bounds;

        // Update display_map wrap_width if changed.
        if let Some(last_layout) = self.last_layout.as_ref() {
            if wrap_width_changed {
                let wrap_width = if !self.soft_wrap {
                    // None to disable wrapping (will use Pixels::MAX)
                    None
                } else {
                    last_layout.wrap_width
                };

                self.display_map.on_layout_changed(wrap_width, cx);
                if self.is_multi_line() {
                    self.mode.update_auto_grow(&self.display_map);
                }
                cx.notify();
            }
        }
    }

    /// Return the active selection's text, borrowed out of the [`Rope`]
    /// the state owns.
    ///
    /// See [`Self::selected_value`] when an owned string is wanted.
    pub fn selected_text(&self) -> RopeSlice<'_> {
        let range_utf16 = self.range_to_utf16(&self.selected_range());
        let range = self.range_from_utf16(&range_utf16);
        self.text.slice(range)
    }

    /// Return the rendered bounds for a UTF-8 byte range in the current input contents.
    ///
    /// Returns `None` when the requested range is not currently laid out or visible.
    pub fn range_to_bounds(&self, range: &Range<usize>) -> Option<Bounds<Pixels>> {
        let Some(last_layout) = self.last_layout.as_ref() else {
            return None;
        };

        let Some(last_bounds) = self.last_bounds else {
            return None;
        };

        use crate::input::display_map::projection::Endpoint;
        let (_, _, start_pos) = self.line_and_position_for_endpoint(
            range.start,
            if range.is_empty() {
                Endpoint::Caret
            } else {
                Endpoint::RangeStart
            },
        );
        let (_, _, end_pos) = self.line_and_position_for_endpoint(
            range.end,
            if range.is_empty() {
                Endpoint::Caret
            } else {
                Endpoint::RangeEnd
            },
        );

        let Some(start_pos) = start_pos else {
            return None;
        };
        let Some(end_pos) = end_pos else {
            return None;
        };

        Some(Bounds::from_corners(
            last_bounds.origin + start_pos,
            last_bounds.origin + end_pos + point(px(0.), last_layout.line_height),
        ))
    }

    /// Replace text in range in silent.
    ///
    /// This will not trigger any UI interaction, such as auto-completion.
    pub(crate) fn replace_text_in_range_silent(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.silent_replace_text = true;
        self.replace_text_in_range(range_utf16, new_text, window, cx);
        self.silent_replace_text = false;
    }

    /// Apply a batch of edits as one atomic history transaction.
    ///
    /// `edits` are `(byte range in the current pre-edit document, replacement)`
    /// pairs.
    pub(crate) fn replace_text_in_ranges(
        &mut self,
        edits: &[(Range<usize>, String)],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_editable() || edits.is_empty() {
            return;
        }

        // Every edit invalidates provider responses for the previous document,
        // including deletion and indentation which do not trigger completion.
        M::hide_context_menu(self, cx);
        M::clear_inline_completion(self, cx);

        // Sort descending by start so applying front-of-vec first edits the
        // highest offsets first, leaving lower offsets unchanged.
        let mut sorted: Vec<(Range<usize>, &str)> = edits
            .iter()
            .map(|(range, text)| (self.normalize_token_range(range.clone()), text.as_str()))
            .collect();
        sorted.sort_by_key(|edit| std::cmp::Reverse(edit.0.start));
        if !self.token_spans().is_empty() {
            let mut merged: Vec<(Range<usize>, &str)> = Vec::with_capacity(sorted.len());
            for (range, text) in sorted {
                if let Some(last) = merged
                    .last_mut()
                    .filter(|last| range.end > last.0.start || range == last.0)
                {
                    last.0.start = range.start.min(last.0.start);
                    last.0.end = range.end.max(last.0.end);
                } else {
                    merged.push((range, text));
                }
            }
            sorted = merged;
        }

        #[cfg(debug_assertions)]
        for pair in sorted.windows(2) {
            debug_assert!(
                pair[1].0.end <= pair[0].0.start,
                "replace_text_in_ranges requires disjoint ranges"
            );
        }

        // Wrap multiple edits in one explicit transaction so they undo as a
        // unit. A single edit records directly, which keeps it eligible for
        // the undo manager's typing coalescing.
        let requested_intent = self.undo_manager.take_pending_intent();
        let selection_before = *self.active_selection();
        let original_selections: Vec<CursorSelection> = self.selections.iter().copied().collect();
        // Snapshot the cursors before applying, so undo can restore them. A
        // delete has already expanded them over the text it removes, so they
        // collapse back to where the user left them.
        let selections_before: Vec<CursorSelection> = self
            .selections
            .iter()
            .map(|selection| {
                Self::collapse_for_intent(
                    requested_intent.unwrap_or(EditIntent::Atomic),
                    *selection,
                    &(selection.start..selection.end),
                )
            })
            .collect();
        let group = sorted.len() > 1;
        if group {
            self.undo_manager.begin_transaction();
        }

        let auto_closed_pairs_before = self.mode.auto_closed_pairs().clone();
        let mut recorded = false;
        // Several edits reach the highlighter as one batch, so it reparses
        // once for the change instead of once per edit.
        let mut highlighter_edits = Vec::new();
        // Token-aware history replay applies several replacements but emits one
        // `Change` at the end, so its edits accumulate instead of being
        // discarded with each silent replacement.
        let accumulate_replay_edits = self.replaying_history && !self.emit_events;
        if !accumulate_replay_edits {
            self.last_input_edits.clear();
        }
        for (range, new_text) in &sorted {
            let old_text = self.text.clone();
            self.mode.adjust_auto_closed_pair(range, new_text.len());
            self.text.replace(range.clone(), new_text);

            M::adjust_annotations(self, range, new_text.len());
            recorded |= self.push_history(
                &old_text,
                range,
                new_text,
                requested_intent,
                selection_before,
                None,
            );

            // Incremental, single-range updates must run per edit.
            self.display_map
                .adjust_folds_for_edit(&old_text, range, new_text);
            self.display_map
                .on_text_changed(&self.text, range, &Rope::from(*new_text), cx);

            let edit = super::mode::replacement_input_edit(&old_text, &self.text, range, new_text);
            if self.emit_events || self.replaying_history {
                self.last_input_edits.push((edit, self.text.clone()));
            }

            if group {
                if self.is_code_editor() {
                    highlighter_edits.push((edit, self.text.clone()));
                }
            } else {
                self.mode.update_highlighter(
                    super::mode::HighlighterUpdate {
                        selected_range: range,
                        old_text: &old_text,
                        new_text: &self.text,
                        change_text: new_text,
                        force: true,
                        input_edit: Some(edit),
                    },
                    window,
                    cx,
                );

                self.update_fold_candidates_incremental(range, new_text);
            }
        }

        if group {
            self.mode
                .update_highlighter_batch::<M>(&highlighter_edits, window, cx);

            // Fold candidates are read from the reparsed document, so each
            // edit is located where it ended up once every edit was applied.
            // `sorted` runs from the highest offset down.
            let mut shift = 0isize;
            for (range, new_text) in sorted.iter().rev() {
                let start = (range.start as isize + shift) as usize;
                self.update_fold_candidates_incremental(&(start..start), new_text);
                shift += new_text.len() as isize - range.len() as isize;
            }

            self.undo_manager.commit_transaction();
        }

        // One observable update per batch instead of one per edit.
        if let Some(diagnostics) = self.mode.diagnostics_mut() {
            diagnostics.reset(&self.text)
        }
        M::refresh_language_features(self, window, cx);
        self.update_search(cx);

        // Compute the resulting cursors.
        // One collapsed cursor per edit, at the end of its inserted text.
        let mut ascending: Vec<(Range<usize>, &str)> = sorted.clone();
        ascending.sort_by_key(|edit| edit.0.start);
        let text_len = self.text.len();
        let mut delta: isize = 0;
        let mut edit_results = Vec::with_capacity(ascending.len());
        for (range, new_text) in &ascending {
            let offset = ((range.start as isize + delta) as usize + new_text.len()).min(text_len);
            edit_results.push((range.clone(), offset));
            delta += new_text.len() as isize - (range.end as isize - range.start as isize);
        }

        let mut used = vec![false; edit_results.len()];
        let mut new_selections: Vec<CursorSelection> = Vec::with_capacity(ascending.len());
        for selection in original_selections {
            if let Ok(index) = edit_results
                .binary_search_by_key(&(selection.start, selection.end), |(range, _)| {
                    (range.start, range.end)
                })
                && !used[index]
            {
                let offset = edit_results[index].1;
                used[index] = true;
                let mut selection = selection;
                selection.place_at(offset, None);
                new_selections.push(selection);
            }
        }
        for (index, (_, offset)) in edit_results.into_iter().enumerate() {
            if !used[index] {
                new_selections.push(CursorSelection::new(
                    self.selections.generate_id(),
                    offset,
                    offset,
                ));
            }
        }
        self.selections.replace_all(new_selections);
        self.selections.merge_overlapping();

        // Record the cursor snapshots for undo/redo restore.
        let selections_after: Vec<CursorSelection> = self.selections.iter().copied().collect();
        if recorded {
            self.undo_manager.record_auto_closed_pairs(
                auto_closed_pairs_before,
                self.mode.auto_closed_pairs().clone(),
            );
            self.undo_manager
                .record_selections(selections_before, selections_after);
        }

        self.ime_marked_range.take();
        self.update_preferred_column();
        self.dismiss_touch_selection(cx);
        if self.is_multi_line() {
            self.mode.update_auto_grow(&self.display_map);
        }
        if self.emit_events {
            self.emit_change(cx);
        }
        cx.notify();
    }

    /// Update fold candidates from tree-sitter syntax tree (full extraction).
    /// Used only on initial load or language changes.
    fn update_fold_candidates(&mut self) {
        if !self.mode.is_folding() {
            return;
        }

        let Some(highlighter_rc) = self.mode.highlighter() else {
            return;
        };

        let highlighter = highlighter_rc.borrow();
        let Some(highlighter) = highlighter.as_ref() else {
            return;
        };

        let fold_ranges = highlighter.fold_ranges(&self.text);
        self.display_map.set_fold_candidates(fold_ranges);
    }

    /// Incrementally update fold candidates after a text edit.
    /// Only traverses the edited region of the syntax tree instead of the full tree.
    fn update_fold_candidates_incremental(&mut self, edit_range: &Range<usize>, new_text: &str) {
        if !self.mode.is_folding() {
            return;
        }

        let Some(highlighter_rc) = self.mode.highlighter() else {
            return;
        };

        let highlighter = highlighter_rc.borrow();
        let Some(highlighter) = highlighter.as_ref() else {
            return;
        };

        // The new byte range in the updated text after the edit
        let new_end = edit_range.start + new_text.len();
        self.display_map.update_fold_candidates_for_edit(
            |range, text| highlighter.fold_ranges_for_edit(range, text),
            edit_range.start..new_end,
            &self.text,
        );
    }
}

impl<M: InputModeKind> EntityInputHandler for InputBaseState<M> {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        adjusted_range.replace(self.range_to_utf16(&range));
        Some(self.text.slice(range).to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range()),
            reversed: self.active_selection().reversed,
        })
    }

    /// Select the given range, as the platform's own text UI asks when it
    /// drives the selection: a native selection handle, or the macOS IME.
    ///
    /// The end that moved becomes the cursor, so dragging the start handle
    /// leaves a reversed selection and keeps the moving end in view. Text and
    /// undo history are left alone.
    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = self.selected_range();
        let was_reversed = self.active_selection().reversed;
        let range = self.range_from_utf16(&range_utf16);
        let range = range.start.min(range.end)..range.start.max(range.end);
        self.set_selected_range(range, cx);

        let range = self.selected_range();
        let reversed = if range == previous {
            was_reversed
        } else if !range.is_empty() && range.end == previous.start {
            true
        } else if !range.is_empty() && range.start == previous.end {
            false
        } else {
            !range.is_empty() && range.start != previous.start && range.end == previous.end
        };
        self.active_selection_mut().reversed = reversed;
        self.scroll_to(self.cursor(), None, cx);
        cx.notify();
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.ime_marked_range
            .map(|range| self.range_to_utf16(&range.into()))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.ime_marked_range = None;
        self.undo_manager.commit_transaction();
        // Unmarking keeps the preedit as committed text, which marked edits
        // never reported. Android keyboards compose every word and often end
        // with only this call (hiding the keyboard, moving the caret).
        if self.ime_original_text.as_ref() == Some(&self.text) {
            self.ime_original_text = None;
        } else if self.ime_original_text.is_some() && self.emit_events {
            self.emit_change(cx);
        }
    }

    /// Replace text in range.
    ///
    /// - If the new text is invalid, it will not be replaced.
    /// - If `range_utf16` is not provided, the current selected range will be used.
    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let requested_intent = self.undo_manager.take_pending_intent();
        if !self.is_editable() {
            return;
        }
        let selection_before = *self.active_selection();
        // Committing a composition ends the transaction it opened, whether or
        // not the platform follows up with `unmark_text`.
        let ends_composition = self.ime_marked_range.is_some();

        self.pause_blink_cursor(cx);

        // NOTE: The normalization keeps the UTF-16 length, but may change the
        // UTF-8 byte length, so all the byte-offset calculations below must
        // use the normalized text.
        let new_text = if self.replaying_history {
            Cow::Borrowed(new_text)
        } else {
            self.normalize_input(new_text)
        };
        let new_text: &str = &new_text;

        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.ime_marked_range.map(|range| {
                let range = self.range_to_utf16(&(range.start..range.end));
                self.range_from_utf16(&range)
            }))
            .unwrap_or(self.selected_range());
        let range = self.normalize_token_range(range);

        // Skip-over as a pure cursor move: a typed closer that already follows
        // the cursor moves past it without touching text or history, so Undo
        // never exposes a transient duplicate. Only for interactive single
        // keystrokes (current selection, no IME mark, not silent replays).
        // `take_pending_intent` above already consumed any request.
        if range.is_empty()
            && range.start == self.cursor()
            && self.ime_marked_range.is_none()
            && !self.silent_replace_text
            && self.is_code_editor()
            && self.selections.is_single()
            && self.active_selection().is_empty()
        {
            if let Some(target) = self.skip_over_target(new_text) {
                self.set_cursor_to(target);
                self.update_preferred_column();
                cx.notify();
                return;
            }
        }

        if self.is_multi_line() {
            let multi_cursor = range_utf16.is_none()
                && self.ime_marked_range.is_none()
                && !self.selections.is_single();
            if multi_cursor {
                self.selections.merge_overlapping();
                let mut edits: Vec<(Range<usize>, String)> = self
                    .selections
                    .iter()
                    .map(|sel| (sel.start..sel.end, new_text.to_string()))
                    .collect();
                edits.sort_by_key(|(range, _)| range.start);
                // One keystroke across several cursors is one batch, committed
                // with the intent of the keystroke so a run of them coalesces
                // into one undo just like single-cursor typing.
                let intent =
                    requested_intent.unwrap_or_else(|| self.typing_intent(&edits, new_text));
                self.undo_manager.begin_transaction_with(intent);
                self.undo_manager.set_pending_intent(intent);
                self.replace_text_in_ranges(&edits, window, cx);
                self.undo_manager.commit_transaction();
            } else {
                if range_utf16.is_some() {
                    self.selections.remove_all_but_active();
                }
                if let Some(intent) = requested_intent {
                    self.undo_manager.set_pending_intent(intent);
                }
                if let Some((open_len, closer)) = self.auto_close_target(&range, new_text) {
                    // One edit keeps the pair atomic even at undo coalescing limits.
                    let replacement = format!("{new_text}{closer}");
                    self.replace_text_in_ranges(&[(range.clone(), replacement)], window, cx);
                    let cursor = range.start + new_text.len();
                    self.mode.track_auto_closed_pair(
                        cursor - open_len..cursor,
                        cursor..cursor + closer.len(),
                    );
                    self.undo_manager
                        .record_auto_closed_pairs_after(self.mode.auto_closed_pairs().clone());
                    self.set_cursor_to(cursor);
                    self.update_preferred_column();
                    self.undo_manager.record_selections(
                        vec![selection_before],
                        self.selections.iter().copied().collect(),
                    );
                } else {
                    self.replace_text_in_ranges(
                        &[(range.clone(), new_text.to_string())],
                        window,
                        cx,
                    );
                }
            }
            if ends_composition {
                self.undo_manager.commit_transaction();
            }

            if !self.silent_replace_text {
                M::on_text_typed(self, &range, new_text, window, cx);
            }
            return;
        }

        // Single-line path
        let old_text = self.text.clone();
        // Token-aware history replay applies several replacements but emits one
        // `Change` at the end, so its edits accumulate instead of being
        // discarded with each silent replacement.
        let accumulate_replay_edits = self.replaying_history && !self.emit_events;
        if !accumulate_replay_edits {
            self.last_input_edits.clear();
        }
        self.mode.adjust_auto_closed_pair(&range, new_text.len());
        self.text.replace(range.clone(), new_text);

        let mut new_offset = (range.start + new_text.len()).min(self.text.len());

        // True if the mask has changed the text, e.g. regrouping the
        // separators or completing a leading dot.
        let mut mask_changed = false;

        if self.is_single_line() {
            let pending_text = self.text.to_string();
            // Check if the new text is valid.
            //
            // Only reject the edit if the old text was valid, to avoid
            // trapping a pre-existing invalid text (e.g. a `default_value`
            // that does not conform), the user can still edit to fix it.
            if !self.replaying_history
                && !self.validated_token_edit
                && !self.is_valid_input(&pending_text, cx)
                && self.is_valid_input(&old_text.to_string(), cx)
            {
                self.text = old_text;
                return;
            }

            if !self.replaying_history && !self.mask_pattern.is_none() {
                let mask_text = self.mask_pattern.mask(&pending_text);
                mask_changed = mask_text.as_str() != pending_text;
                self.text = Rope::from(mask_text.as_str());
                let new_text_len =
                    (new_text.len() + mask_text.len()).saturating_sub(pending_text.len());
                new_offset = (range.start + new_text_len).min(mask_text.len());
            }
        }

        if mask_changed {
            // Masking rewrites the whole document, so ranges recorded against
            // the old text no longer point at anything.
            M::reset_annotations(self);
        } else {
            M::adjust_annotations(self, &range, new_text.len());
        }
        if mask_changed {
            // A segment-based history entry no longer matches the masked
            // document, record a whole-document change instead, so that
            // undo/redo can restore the text exactly.
            self.push_history(
                &old_text,
                &(0..old_text.len()),
                &self.text.to_string(),
                Some(EditIntent::Atomic),
                selection_before,
                Some((new_offset..new_offset).into()),
            );
        } else {
            self.push_history(
                &old_text,
                &range,
                &new_text,
                requested_intent,
                selection_before,
                None,
            );
        }
        if let Some(diagnostics) = self.mode.diagnostics_mut() {
            diagnostics.reset(&self.text)
        }
        // Adjust folds before updating wrap map: remove overlapping folds and shift others
        self.display_map
            .adjust_folds_for_edit(&old_text, &range, new_text);
        self.display_map
            .on_text_changed(&self.text, &range, &Rope::from(new_text), cx);

        let edit = if mask_changed {
            Self::full_input_edit(&old_text, &self.text)
        } else {
            super::mode::replacement_input_edit(&old_text, &self.text, &range, new_text)
        };
        if self.emit_events || self.replaying_history {
            self.last_input_edits.push((edit, self.text.clone()));
        }

        self.mode.update_highlighter::<M>(
            super::mode::HighlighterUpdate {
                selected_range: &range,
                old_text: &old_text,
                new_text: &self.text,
                change_text: &new_text,
                force: true,
                input_edit: Some(edit),
            },
            window,
            cx,
        );

        self.update_fold_candidates_incremental(&range, new_text);
        M::refresh_language_features(self, window, cx);
        self.set_cursor_to(new_offset);
        self.ime_marked_range.take();
        // A commit ends the IME composition: macOS delivers `insertText:` for
        // the confirmed candidate without a following `unmarkText`, so close
        // the transaction here. Leaving it open would keep merging every later
        // edit into the same change, which then carries the text and selection
        // of the first composition.
        if ends_composition {
            self.undo_manager
                .record_selections(vec![selection_before], vec![*self.active_selection()]);
            self.undo_manager.commit_transaction();
        }
        self.update_preferred_column();
        self.update_search(cx);
        self.dismiss_touch_selection(cx);
        if self.is_multi_line() {
            self.mode.update_auto_grow(&self.display_map);
        }
        if !self.silent_replace_text {
            M::on_text_typed(self, &range, &new_text, window, cx);
        }
        if self.emit_events {
            self.emit_change(cx);
        }
        cx.notify();
    }

    /// Mark text is the IME temporary insert on typing.
    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let requested_intent = self.undo_manager.take_pending_intent();
        if !self.is_editable() {
            return;
        }
        let selection_before = *self.active_selection();

        let starts_composition = self.ime_marked_range.is_none();
        if starts_composition {
            // Even a canceled preedit separates the typing gestures on either
            // side; its no-op transaction must not reconnect those gestures.
            self.undo_manager.break_transaction_coalescing();
            self.undo_manager.begin_transaction();
        }

        // Collapse any extra cursors so we never leave stale secondary cursors behind.
        self.selections.remove_all_but_active();

        M::reset_language_features(self);

        // See the same NOTE in `replace_text_in_range`.
        let new_text = if self.replaying_history {
            Cow::Borrowed(new_text)
        } else {
            self.normalize_input(new_text)
        };
        let new_text: &str = &new_text;

        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.ime_marked_range.map(|range| {
                let range = self.range_to_utf16(&(range.start..range.end));
                self.range_from_utf16(&range)
            }))
            .unwrap_or(self.selected_range());
        let range = self.normalize_token_range(range);

        let auto_closed_pairs_before = self.mode.auto_closed_pairs().clone();
        let old_text = self.text.clone();
        // Marked text does not emit `InputEvent::Change`, so no edit belongs to
        // the most recent change after it.
        self.last_input_edits.clear();
        self.mode.adjust_auto_closed_pair(&range, new_text.len());
        self.text.replace(range.clone(), new_text);

        if self.is_single_line() {
            let pending_text = self.text.to_string();
            // See the same NOTE in `replace_text_in_range`.
            if !self.replaying_history
                && !self.validated_token_edit
                && !self.is_valid_input(&pending_text, cx)
                && self.is_valid_input(&old_text.to_string(), cx)
            {
                self.text = old_text;
                if starts_composition {
                    self.undo_manager.commit_transaction();
                }
                return;
            }
        }

        if self.ime_original_text.is_none() {
            self.ime_original_text = Some(old_text.clone());
        }
        M::adjust_annotations(self, &range, new_text.len());
        if let Some(diagnostics) = self.mode.diagnostics_mut() {
            diagnostics.reset(&self.text)
        }
        // Adjust folds before updating wrap map: remove overlapping folds and shift others
        self.display_map
            .adjust_folds_for_edit(&old_text, &range, new_text);
        self.display_map
            .on_text_changed(&self.text, &range, &Rope::from(new_text), cx);

        self.mode.update_highlighter::<M>(
            super::mode::HighlighterUpdate {
                selected_range: &range,
                old_text: &old_text,
                new_text: &self.text,
                change_text: &new_text,
                force: true,
                input_edit: None,
            },
            window,
            cx,
        );

        self.update_fold_candidates_incremental(&range, new_text);
        M::refresh_language_features(self, window, cx);
        if new_text.is_empty() {
            // Cancel selection, when cancel IME input.
            self.set_cursor_to(range.start);
            self.ime_marked_range = None;
        } else {
            self.ime_marked_range = Some((range.start..range.start + new_text.len()).into());
            let new_range = new_selected_range_utf16
                .as_ref()
                .map(|range_utf16| {
                    let new_text = Rope::from(new_text);
                    range.start + new_text.offset_utf16_to_offset(range_utf16.start)
                        ..range.start + new_text.offset_utf16_to_offset(range_utf16.end)
                })
                .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
            self.set_selection(new_range.start, new_range.end);
        }
        if self.is_multi_line() {
            self.mode.update_auto_grow(&self.display_map);
        }
        if self.push_history(
            &old_text,
            &range,
            new_text,
            requested_intent,
            selection_before,
            Some(*self.active_selection()),
        ) {
            self.undo_manager
                .record_selections(vec![selection_before], vec![*self.active_selection()]);
            self.undo_manager.record_auto_closed_pairs(
                auto_closed_pairs_before,
                self.mode.auto_closed_pairs().clone(),
            );
        }
        if new_text.is_empty() {
            self.undo_manager.commit_transaction();
        }
        cx.notify();
    }

    /// Used to position IME candidates.
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        let line_height = last_layout.line_height;
        let line_number_width = last_layout.line_number_width;
        let range = self.range_from_utf16(&range_utf16);

        let mut start_origin = None;
        let mut end_origin = None;
        let line_number_origin = point(line_number_width, px(0.));
        let mut y_offset = last_layout.visible_top;

        let start_row = self
            .text
            .offset_to_point(range.start.min(self.text.len()))
            .row;
        let end_row = self
            .text
            .offset_to_point(range.end.min(self.text.len()))
            .row;

        for (vi, line) in last_layout.lines.iter().enumerate() {
            if start_origin.is_some() && end_origin.is_some() {
                break;
            }

            let buffer_line = last_layout.visible_buffer_lines[vi];
            let index_offset = last_layout.visible_line_byte_offsets[vi];

            if start_origin.is_none() && buffer_line == self.display_map.projected_owner(start_row)
            {
                let local_offset = range.start.saturating_sub(index_offset).min(line.len());
                let p = line
                    .position_for_endpoint(
                        local_offset,
                        last_layout,
                        false,
                        if range.is_empty() {
                            crate::input::display_map::projection::Endpoint::Caret
                        } else {
                            crate::input::display_map::projection::Endpoint::RangeStart
                        },
                    )
                    .unwrap_or_else(|| point(px(0.), px(0.)));
                start_origin = Some(p + point(px(0.), y_offset));
            }

            if end_origin.is_none() && buffer_line == self.display_map.projected_owner(end_row) {
                let local_offset = range.end.saturating_sub(index_offset).min(line.len());
                let p = line
                    .position_for_endpoint(
                        local_offset,
                        last_layout,
                        self.editor_paint().is_some() && !range.is_empty(),
                        if range.is_empty() {
                            crate::input::display_map::projection::Endpoint::Caret
                        } else {
                            crate::input::display_map::projection::Endpoint::RangeEnd
                        },
                    )
                    .unwrap_or_else(|| point(px(0.), px(0.)));
                end_origin = Some(p + point(px(0.), y_offset));
            }

            y_offset += line.size(line_height).height;
        }

        let start_origin = start_origin.or_else(|| {
            let offset = self.last_cursor.or(Some(self.cursor()))?;
            let (_, _, origin) = self.line_and_position_for_offset(offset);
            origin.map(|origin| origin - line_number_origin)
        })?;
        let mut end_origin = end_origin.unwrap_or(start_origin);
        // Ensure at same line.
        end_origin.y = start_origin.y;
        if end_origin.x <= start_origin.x {
            end_origin.x = start_origin.x + px(1.);
        }

        Some(Bounds::from_corners(
            bounds.origin + line_number_origin + start_origin,
            // + line_height for show IME panel under the cursor line.
            bounds.origin + line_number_origin + point(end_origin.x, end_origin.y + line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let last_layout = self.last_layout.as_ref()?;
        // The same origin as `resolve_mouse_position`: past the gutter, at the first
        // visible line.
        // The scrollable text origin can be outside the editor after reflow.
        // Clip in viewport coordinates, then localize against the painted origin.
        if !self.input_bounds.contains(&point) {
            return None;
        }
        let inner_point = point
            - self.last_bounds?.origin
            - gpui::point(last_layout.line_number_width, last_layout.visible_top);

        let mut line_top = px(0.);
        for (vi, line) in last_layout.lines.iter().enumerate() {
            let offset = last_layout.visible_line_byte_offsets[vi];
            let line_point = inner_point - gpui::point(px(0.), line_top);
            if let Some(utf8_index) = line.index_for_position(line_point, last_layout) {
                return Some(self.offset_to_utf16(self.resolve_index(offset + utf8_index)));
            }
            line_top += line.size(last_layout.line_height).height;
        }

        None
    }

    fn text_input_configuration(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> TextInputConfiguration {
        InputBaseState::text_input_configuration(self)
    }
}

impl<M: InputModeKind> Focusable for InputBaseState<M> {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl<M: InputModeKind> Render for InputBaseState<M> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Before anything reads it: the element resolves this style during
        // layout and paint, and both happen after this call in the same frame.
        self.editor_style = self
            .projected_editor_style
            .resolved(&crate::Theme::global(cx).tokens);
        let entity = cx.entity();
        let text_runs = self.accessibility_text_runs(window);
        if self._pending_update {
            self.mode.update_highlighter::<M>(
                super::mode::HighlighterUpdate {
                    selected_range: &(0..0),
                    old_text: &self.text,
                    new_text: &self.text,
                    change_text: "",
                    force: false,
                    input_edit: None,
                },
                window,
                cx,
            );

            self.update_fold_candidates();
            M::refresh_language_features(self, window, cx);
            self._pending_update = false;
        }

        let element = div()
            .id(EDITOR_ELEMENT_ID)
            .key_context(CONTEXT)
            .when_some(self.accessibility.as_ref(), |this, accessibility| {
                this.when_some(accessibility.role, |this, role| this.role(role))
                    .when_some(accessibility.label.clone(), |this, label| {
                        this.aria_label(label)
                    })
                    .when_some(accessibility.placeholder.clone(), |this, placeholder| {
                        this.aria_placeholder(placeholder)
                    })
                    .when_some(accessibility.accessibility_id.clone(), |this, id| {
                        this.accessibility_id(id)
                    })
                    .when(
                        accessibility.value_exposed
                            && !self.masked
                            && (window.is_a11y_active() || cfg!(feature = "test-support")),
                        |this| this.aria_value(self.text.to_string()),
                    )
                    .when_some(text_runs, |this, runs| {
                        this.a11y_synthetic_children(move |builder| runs.push(builder))
                    })
                    .on_a11y_action(gpui::AccessibleAction::Focus, {
                        let entity = entity.clone();
                        move |_, window, cx| {
                            entity.update(cx, |state, cx| {
                                state.handle_accessibility_focus(window, cx)
                            });
                        }
                    })
                    .when(self.is_editable(), |this| {
                        this.on_a11y_action(gpui::AccessibleAction::SetValue, {
                            let entity = entity.clone();
                            move |data, window, cx| {
                                entity.update(cx, |state, cx| {
                                    state.handle_accessibility_set_value(data, window, cx)
                                });
                            }
                        })
                    })
            })
            .when(self.is_editable(), |this| {
                this.on_action(window.listener_for(&entity, InputBaseState::backspace))
                    .on_action(window.listener_for(&entity, InputBaseState::delete))
                    .on_action(
                        window.listener_for(&entity, InputBaseState::delete_to_beginning_of_line),
                    )
                    .on_action(window.listener_for(&entity, InputBaseState::delete_to_end_of_line))
                    .on_action(window.listener_for(&entity, InputBaseState::delete_previous_word))
                    .on_action(window.listener_for(&entity, InputBaseState::delete_next_word))
                    .on_action(window.listener_for(&entity, InputBaseState::enter))
                    .on_action(window.listener_for(&entity, InputBaseState::escape))
                    .on_action(window.listener_for(&entity, InputBaseState::paste))
                    .on_action(window.listener_for(&entity, InputBaseState::cut))
                    .on_action(window.listener_for(&entity, InputBaseState::undo))
                    .on_action(window.listener_for(&entity, InputBaseState::redo))
                    .when(self.is_multi_line(), |this| {
                        this.on_action(window.listener_for(&entity, InputBaseState::indent_inline))
                            .on_action(window.listener_for(&entity, InputBaseState::outdent_inline))
                            .on_action(window.listener_for(&entity, InputBaseState::indent_block))
                            .on_action(window.listener_for(&entity, InputBaseState::outdent_block))
                    })
            })
            .on_action(window.listener_for(&entity, InputBaseState::left))
            .on_action(window.listener_for(&entity, InputBaseState::right))
            .on_action(window.listener_for(&entity, InputBaseState::select_left))
            .on_action(window.listener_for(&entity, InputBaseState::select_right))
            .when(self.is_multi_line(), |this| {
                this.on_action(window.listener_for(&entity, InputBaseState::up))
                    .on_action(window.listener_for(&entity, InputBaseState::down))
                    .on_action(window.listener_for(&entity, InputBaseState::select_up))
                    .on_action(window.listener_for(&entity, InputBaseState::select_down))
                    .on_action(window.listener_for(&entity, InputBaseState::page_up))
                    .on_action(window.listener_for(&entity, InputBaseState::page_down))
                    .on_action(window.listener_for(&entity, InputBaseState::add_cursor_above))
                    .on_action(window.listener_for(&entity, InputBaseState::add_cursor_below))
            })
            .on_action(window.listener_for(&entity, InputBaseState::on_action_select_all))
            .on_action(window.listener_for(&entity, InputBaseState::select_to_start_of_line))
            .on_action(window.listener_for(&entity, InputBaseState::select_to_end_of_line))
            .on_action(window.listener_for(&entity, InputBaseState::select_to_previous_word))
            .on_action(window.listener_for(&entity, InputBaseState::select_to_next_word))
            .on_action(window.listener_for(&entity, InputBaseState::home))
            .on_action(window.listener_for(&entity, InputBaseState::end))
            .on_action(window.listener_for(&entity, InputBaseState::move_to_start))
            .on_action(window.listener_for(&entity, InputBaseState::move_to_end))
            .on_action(window.listener_for(&entity, InputBaseState::move_to_previous_word))
            .on_action(window.listener_for(&entity, InputBaseState::move_to_next_word))
            .on_action(window.listener_for(&entity, InputBaseState::select_to_start))
            .on_action(window.listener_for(&entity, InputBaseState::select_to_end))
            .on_action(window.listener_for(&entity, InputBaseState::show_character_palette))
            .on_action({
                let entity = entity.clone();
                move |_: &ActivateToken, window, cx| {
                    let state = entity.read(cx);
                    let activation = state
                        .token_spans()
                        .iter()
                        .find(|span| span.range() == state.selected_range())
                        .and_then(|span| {
                            state.range_to_bounds(&span.range()).and_then(|bounds| {
                                state.token_activation(
                                    span.range().start,
                                    bounds,
                                    gpui::ClickEvent::Keyboard(gpui::KeyboardClickEvent {
                                        bounds,
                                        ..Default::default()
                                    }),
                                )
                            })
                        });
                    if let Some((listener, event)) = activation {
                        listener(&event, window, cx);
                    }
                }
            })
            .on_action(window.listener_for(&entity, InputBaseState::copy))
            .on_action(window.listener_for(&entity, InputBaseState::on_action_search))
            .on_action(window.listener_for(&entity, InputBaseState::on_action_replace))
            .on_mouse_down(
                MouseButton::Left,
                window.listener_for(&entity, InputBaseState::on_mouse_down),
            )
            .on_mouse_down(
                MouseButton::Right,
                window.listener_for(&entity, InputBaseState::on_mouse_down),
            )
            .on_mouse_up(
                MouseButton::Left,
                window.listener_for(&entity, InputBaseState::on_mouse_up),
            )
            .on_mouse_up(
                MouseButton::Right,
                window.listener_for(&entity, InputBaseState::on_mouse_up),
            )
            .on_mouse_move(window.listener_for(&entity, InputBaseState::on_mouse_move))
            .on_scroll(window.listener_for(&entity, InputBaseState::on_scroll_wheel))
            .when(self.is_multi_line() && !self.disabled, |this| {
                this.on_modifiers_changed(cx.listener(|_, _, _, cx| cx.notify()))
            })
            .when(!self.disabled, |this| {
                if self.is_multi_line() && window.modifiers().alt {
                    this.cursor_crosshair()
                } else {
                    this.cursor_text()
                }
            })
            .flex_1()
            // A single line fills the frame and sits at its vertical center,
            // so the frame needs no layout of its own to hold it. It is never
            // shorter than its line, though: this root clips (overflow-x
            // hidden clips both axes), and a frame whose padding leaves less
            // than a line, like Component's Input, would cut off descenders.
            .map(|this| {
                if self.is_multi_line() {
                    this.h_full()
                } else {
                    this.min_h_full().flex().items_center()
                }
            })
            .flex_grow_1()
            .overflow_x_hidden()
            .when(self.is_multi_line(), |this| {
                this.pt(self.editor_paddings.top)
                    .pr(self.editor_paddings.right)
                    .pb(self.editor_paddings.bottom)
                    .pl(self.editor_paddings.left)
            })
            .child(TextElement::new(entity.clone()).placeholder(self.placeholder.clone()))
            .when(self.shows_scrollbar(), |this| {
                this.child(EditorScrollbar::new(entity.clone()))
            });

        // Actions only one mode handles are registered by that mode, where
        // `Self` is concrete enough to name its own entity type.
        M::register_actions(element, &entity, window)
            .test_support()
            .track_focus(&self.focus_handle)
    }
}

/// Methods that only a single-line input offers.
impl InputBaseState<crate::input::InputMode> {
    /// Create a single-line text input state.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_in_mode(window, cx)
    }

    /// Set a custom step function of the [`super::NumberInput`].
    ///
    /// The `f` receives the current value and the [`StepAction`], and returns
    /// the step to apply, so the step can vary with the value.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // At the boundary 1.0 the step is 0.1 going down and 0.5 going up.
    /// InputState::new(window, cx).step_by(|value, action, _cx| match action {
    ///     StepAction::Increment => if value < 1.0 { 0.1 } else { 0.5 },
    ///     StepAction::Decrement => if value <= 1.0 { 0.1 } else { 0.5 },
    /// })
    /// ```
    pub fn step_by(mut self, f: impl Fn(f64, StepAction, &mut App) -> f64 + 'static) -> Self {
        self.number_step = Some(NumberStep::by_value(f));
        self
    }

    /// Set with password masked state.
    pub fn masked(mut self, masked: bool) -> Self {
        self.masked = masked;
        self
    }

    /// Set the password masked state of the input field.
    pub fn set_masked(&mut self, masked: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.masked = masked;
        cx.notify();
    }

    /// Set the regular expression pattern of the input field.
    pub fn pattern(mut self, pattern: regex::Regex) -> Self {
        self.pattern = Some(pattern);
        self
    }

    /// Set the regular expression pattern of the input field with reference.
    pub fn set_pattern(
        &mut self,
        pattern: regex::Regex,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.pattern = Some(pattern);
    }

    /// Set the validation function of the input field.
    pub fn validate(mut self, f: impl Fn(&str, &mut App) -> bool + 'static) -> Self {
        self.validate = Some(Box::new(f));
        self
    }

    pub fn set_validator(
        &mut self,
        validate: impl Fn(&str, &mut App) -> bool + 'static,
        _cx: &mut Context<Self>,
    ) {
        self.validate = Some(Box::new(validate));
    }

    /// Set the step value of the [`super::NumberInput`] for increment/decrement.
    ///
    /// If any of `step`, `min`, `max` is set, the [`super::NumberInput`] will
    /// update the value internally (step by `step`, default 1, clamp to the
    /// `min`/`max` range and emit [`InputEvent::Change`]) instead of emitting
    /// [`super::NumberInputEvent::Step`].
    ///
    /// See also [`Self::step_by`] to calculate the step value
    /// based on the current value.
    pub fn step(mut self, step: impl Into<NumberStep>) -> Self {
        self.number_step = Some(step.into());
        self
    }

    /// Set the minimum value of the [`super::NumberInput`].
    ///
    /// The value will be clamped to the minimum value on stepping and on
    /// blur (only if the clamped value passes the `pattern`/`validate` check).
    /// See also [`Self::step`].
    pub fn min(mut self, min: f64) -> Self {
        self.number_min = Some(min);
        self
    }

    /// Set the maximum value of the [`super::NumberInput`].
    ///
    /// The value will be clamped to the maximum value on stepping and on
    /// blur (only if the clamped value passes the `pattern`/`validate` check).
    /// See also [`Self::step`].
    pub fn max(mut self, max: f64) -> Self {
        self.number_max = Some(max);
        self
    }

    /// Update the step value after construction, `None` to fall back to
    /// emitting [`super::NumberInputEvent::Step`] (if `min`, `max` are unset).
    ///
    /// See [`Self::step`] and [`Self::step_by`].
    pub fn set_step(
        &mut self,
        step: impl Into<Option<NumberStep>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        self.number_step = step.into();
    }

    /// Update the minimum value after construction. See [`Self::min`].
    pub fn set_min(&mut self, min: Option<f64>, _: &mut Window, _: &mut Context<Self>) {
        self.number_min = min;
    }

    /// Update the maximum value after construction. See [`Self::max`].
    pub fn set_max(&mut self, max: Option<f64>, _: &mut Window, _: &mut Context<Self>) {
        self.number_max = max;
    }

    /// Set true to show spinner at the input right.
    pub fn set_loading(&mut self, loading: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.loading = loading;
        cx.notify();
    }
}

/// Methods shared by the two multi-line modes, and reachable on neither a
/// single-line input nor anything else.
impl<M: crate::input::MultiLineMode> InputBaseState<M> {
    /// Set whether text taller than the input stretches past its top and
    /// bottom and springs back when precise trackpad or Magic Mouse scrolling
    /// or its momentum runs into an end. Ordinary line-wheel scrolling does
    /// not bounce. The default follows
    /// [`crate::ScrollBounce::enabled_by_default`]: on on iOS, Android and
    /// macOS, off elsewhere. Under reduced motion the text stops at its ends
    /// without stretching.
    pub fn scroll_bounce(mut self, scroll_bounce: bool) -> Self {
        self.scroll_bounce = scroll_bounce;
        self
    }

    /// Update whether the text stretches past its ends, see
    /// [`InputBaseState::scroll_bounce`]. Turning it off drops a stretch in
    /// progress.
    pub fn set_scroll_bounce(&mut self, scroll_bounce: bool, cx: &mut Context<Self>) {
        if self.scroll_bounce != scroll_bounce {
            self.scroll_bounce = scroll_bounce;
            self.overscroll.reset();
            cx.notify();
        }
    }

    /// Whether the built-in search panel and its shortcut are enabled. Off by
    /// default, on for the code editor.
    ///
    /// This only concerns the panel. An input that is not searchable still
    /// answers [`InputBaseState::set_search_query`] and the other search
    /// methods, and lets `Ctrl-F` / `Cmd-F` bubble up to its ancestors, so an
    /// application can put its own search UI on top of the same engine.
    pub fn searchable(mut self, searchable: bool) -> Self {
        self.searchable = searchable;
        self
    }

    /// See [`InputBaseState::searchable`].
    pub fn set_searchable(&mut self, searchable: bool, cx: &mut Context<Self>) {
        self.searchable = searchable;
        cx.notify();
    }

    /// Set the soft wrap mode, default is true.
    #[doc(hidden)]
    pub fn soft_wrap(mut self, wrap: bool) -> Self {
        self.soft_wrap = wrap;
        self
    }

    /// Update the soft wrap mode, default is true.
    pub fn set_soft_wrap(&mut self, wrap: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.soft_wrap = wrap;
        if wrap {
            let wrap_width = self
                .last_layout
                .as_ref()
                .and_then(|b| b.wrap_width)
                .unwrap_or(self.input_bounds.size.width);

            self.display_map.on_layout_changed(Some(wrap_width), cx);

            // Reset scroll to left 0
            let mut offset = self.scroll_handle.offset();
            offset.x = px(0.);
            self.scroll_handle.set_offset(offset);
        } else {
            self.display_map.on_layout_changed(None, cx);
        }
        cx.notify();
    }

    /// Set how soft-wrapped continuation lines are indented, default is [`WrappingIndent::Same`]
    #[doc(hidden)]
    pub fn wrapping_indent(mut self, wrapping_indent: WrappingIndent) -> Self {
        self.wrapping_indent = wrapping_indent;
        self
    }

    /// Update how soft-wrapped continuation lines are indented.
    pub fn set_wrapping_indent(
        &mut self,
        wrapping_indent: WrappingIndent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.wrapping_indent = wrapping_indent;
        self.display_map.set_wrapping_indent(wrapping_indent, cx);
        cx.notify();
    }
}

/// Methods that only ordinary multi-line text offers.
impl InputBaseState<crate::input::TextareaMode> {
    /// Create a multi-line text state.
    ///
    /// Being multi-line is carried by the mode, not by the layout, so the
    /// default plain-text layout needs no adjustment here.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_in_mode(window, cx)
    }

    pub fn set_auto_grow(&mut self, min_rows: usize, max_rows: usize, cx: &mut Context<Self>) {
        self.mode = LayoutMode::auto_grow(min_rows, max_rows.max(min_rows));
        self.mode.update_auto_grow(&self.display_map);
        cx.notify();
    }

    /// Set the number of rows for the multi-line Textarea.
    ///
    /// This is only used when `multi_line` is set to true. The input is at least
    /// this many lines tall.
    ///
    /// default: 1
    #[doc(hidden)]
    pub fn rows(mut self, rows: usize) -> Self {
        match &mut self.mode {
            LayoutMode::PlainText { rows: r, .. } | LayoutMode::CodeEditor { rows: r, .. } => {
                *r = rows
            }
            LayoutMode::AutoGrow {
                max_rows: max_r,
                rows: r,
                ..
            } => {
                *r = rows;
                *max_r = rows;
            }
        }
        self
    }

    pub fn set_rows(&mut self, rows: usize, cx: &mut Context<Self>) {
        match &mut self.mode {
            LayoutMode::PlainText { rows: value, .. }
            | LayoutMode::CodeEditor { rows: value, .. } => *value = rows,
            LayoutMode::AutoGrow {
                rows: value,
                max_rows,
                ..
            } => {
                *value = rows;
                *max_rows = rows;
            }
        }
        cx.notify();
    }

    /// Grow with the content from `min_rows` through `max_rows`.
    pub fn auto_grow(mut self, min_rows: usize, max_rows: usize) -> Self {
        self.mode = LayoutMode::auto_grow(min_rows, max_rows);
        self
    }
}

/// Methods that only a source-code editor offers.
impl InputBaseState<crate::input::EditorMode> {
    /// Create a source-code editor state.
    ///
    /// Default options: line numbers on, tab size 2 with soft tabs, indent
    /// guides on, multi-line, and search enabled. Set the language for syntax
    /// highlighting with [`Self::language`]; without one the text is shown
    /// unhighlighted.
    ///
    /// The editor aims at simple code editing or display, not at being a
    /// full-featured code editor. It offers syntax highlighting, auto indent,
    /// line numbers, and handles large text up to about 50K lines.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut state = Self::new_in_mode(window, cx);
        state.mode = LayoutMode::code_editor(super::EditorLanguage::new(cx));
        state.searchable = true;
        state
    }

    /// Set the language to highlight, e.g. `"rust"`.
    ///
    /// See [`Self::set_highlighter`] to change it after construction.
    pub fn language(mut self, language: impl Into<SharedString>) -> Self {
        if let LayoutMode::CodeEditor {
            language: l,
            highlighter,
            ..
        } = &mut self.mode
        {
            l.set_name(language.into());
            *highlighter.borrow_mut() = None;
        }
        self
    }

    /// The current language name, e.g. `"rust"`.
    pub fn language_name(&self) -> SharedString {
        match &self.mode {
            LayoutMode::CodeEditor { language, .. } => language.name(),
            _ => SharedString::default(),
        }
    }

    /// Set enable/disable code folding.
    ///
    /// Default: true
    #[doc(hidden)]
    pub fn folding(mut self, folding: bool) -> Self {
        if let LayoutMode::CodeEditor { folding: f, .. } = &mut self.mode {
            *f = folding;
        }
        self
    }

    /// Set code folding at runtime.
    ///
    /// When disabling, all existing folds are cleared.
    pub fn set_folding(&mut self, folding: bool, _: &mut Window, cx: &mut Context<Self>) {
        if let LayoutMode::CodeEditor { folding: f, .. } = &mut self.mode {
            *f = folding;
        }
        if !folding {
            self.display_map.clear_folds();
        }
        cx.notify();
    }

    /// Unfold any folded ranges that hide the given position.
    ///
    /// Use this to reveal a position before acting on it (e.g. before
    /// [`Self::set_cursor_position`], which stops at a fold boundary),
    /// without touching folds elsewhere in the buffer. Fold candidates are
    /// kept, so the opened ranges can be folded again from the gutter.
    ///
    /// A fold keeps its own first and last line visible, so a position on
    /// either of them opens nothing. Nested folds all open, since opening
    /// only the outermost would leave the position hidden.
    ///
    /// Returns whether any fold was opened.
    pub fn unfold_at(&mut self, position: impl Into<Position>, cx: &mut Context<Self>) -> bool {
        let offset = self.text.position_to_offset(&position.into());
        let line = self.text.offset_to_point(offset).row;
        // A fold hides start_line + 1 ..= end_line - 1, so a line is hidden
        // exactly when some folded range strictly contains it.
        let covering: Vec<usize> = self
            .display_map
            .folded_ranges()
            .iter()
            .filter(|fold| line > fold.start_line && line < fold.end_line)
            .map(|fold| fold.start_line)
            .collect();
        if covering.is_empty() {
            return false;
        }

        for start_line in covering {
            self.display_map.set_folded(start_line, false);
        }
        cx.notify();
        true
    }

    /// Horizontal offset from the content area's left edge to unscrolled source text.
    ///
    /// Includes the laid-out gap, line numbers, folding controls, and gutter
    /// decorations. Excludes input-frame padding, horizontal scrolling, and
    /// text alignment. For left-aligned text, add this to the content area's
    /// origin and the horizontal scroll offset to recover the text origin.
    ///
    /// Returns `None` before the first layout. Like [`Self::range_to_bounds`],
    /// this describes the most recent layout; render again after changing
    /// gutter settings or typography before using it to align a static view.
    pub fn text_origin_offset(&self) -> Option<Pixels> {
        self.last_layout
            .as_ref()
            .map(|layout| layout.line_number_width)
    }

    /// Set enable/disable line number.
    #[doc(hidden)]
    pub fn line_number(mut self, line_number: bool) -> Self {
        if let LayoutMode::CodeEditor { line_number: l, .. } = &mut self.mode {
            *l = line_number;
        }
        self
    }

    /// Set line number.
    pub fn set_line_number(&mut self, line_number: bool, _: &mut Window, cx: &mut Context<Self>) {
        if let LayoutMode::CodeEditor { line_number: l, .. } = &mut self.mode {
            if *l == line_number {
                return;
            }
            *l = line_number;
        }
        cx.notify();
    }

    /// Set enable/disable automatic closing brackets and quotes.
    ///
    /// When enabled, typing an opener from [`LanguageConfig::auto_closing_pairs`](crate::input::language_config::LanguageConfig::auto_closing_pairs) inserts the
    /// matching closer and places the cursor inside. Typing a closer that is
    /// already present just moves past it. Default: true
    #[doc(hidden)]
    pub fn auto_close(mut self, auto_close: bool) -> Self {
        self.mode.set_auto_close(auto_close);
        self
    }

    /// Set automatic closing brackets and quotes at runtime.
    pub fn set_auto_close(&mut self, auto_close: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.mode.set_auto_close(auto_close);
        cx.notify();
    }

    /// Set enable/disable smart indent on Enter.
    ///
    /// When enabled, Enter uses structural brackets and
    /// [`LanguageConfig::indentation_rules`](crate::input::language_config::LanguageConfig::indentation_rules) to choose indentation.
    /// Default: true
    #[doc(hidden)]
    pub fn smart_indent(mut self, smart_indent: bool) -> Self {
        self.mode.set_smart_indent(smart_indent);
        self
    }

    /// Set smart indent on Enter at runtime.
    pub fn set_smart_indent(&mut self, smart_indent: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.mode.set_smart_indent(smart_indent);
        cx.notify();
    }
}
