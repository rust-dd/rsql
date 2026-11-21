use std::{ops::Range, rc::Rc, sync::Arc};

use gpui::{AnyElement, Context, HighlightStyle, Hsla, SharedString, Window};
use ropey::Rope;

use super::{EditorState, FoldRange, InputEdit};
use crate::SemanticThemeTokens;

/// Resolves semantic highlight names into renderable GPUI styles.
///
/// Base deliberately knows nothing about a concrete syntax theme. UI crates and
/// applications can provide any resolver, independently of their parser.
pub trait HighlightStyleResolver: Send + Sync {
    fn style(&self, name: &str) -> Option<HighlightStyle>;
}

#[derive(Default)]
struct NoHighlightStyles;

impl HighlightStyleResolver for NoHighlightStyles {
    fn style(&self, _: &str) -> Option<HighlightStyle> {
        None
    }
}

/// Parser-independent syntax highlighting seam consumed by the Base editor.
///
/// Implementations own parsing, incremental state, and language-specific
/// behavior. Base only asks for styled ranges and fold candidates.
pub trait InputHighlighter {
    fn language(&self) -> SharedString;

    fn update(
        &mut self,
        edit: Option<InputEdit>,
        text: &Rope,
        folding: bool,
        window: &mut Window,
        cx: &mut Context<EditorState>,
    );

    /// Apply several edits made as one change, such as typing with multiple
    /// cursors. Each entry is an edit with the text as it stood right after
    /// it, in the order the edits were applied.
    ///
    /// The default hands each edit to [`Self::update`] in turn. Override it to
    /// reparse once for the whole change.
    fn update_batch(
        &mut self,
        edits: &[(InputEdit, Rope)],
        folding: bool,
        window: &mut Window,
        cx: &mut Context<EditorState>,
    ) {
        for (edit, text) in edits {
            self.update(Some(*edit), text, folding, window, cx);
        }
    }

    /// Return ordered, non-overlapping style runs that fully cover `range`.
    /// Use [`HighlightStyle::default`] for text without a semantic style.
    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)>;

    fn fold_ranges(&self, text: &Rope) -> Vec<FoldRange>;

    fn fold_ranges_for_edit(&self, range: Range<usize>, text: &Rope) -> Vec<FoldRange> {
        let _ = range;
        self.fold_ranges(text)
    }
}

pub type InputHighlighterFactory = Rc<dyn Fn(&str) -> Option<Box<dyn InputHighlighter>>>;
pub type SharedHighlightStyleResolver = Arc<dyn HighlightStyleResolver>;
pub type FoldIconRenderer = Rc<dyn Fn(usize, bool) -> AnyElement>;
pub type FoldPlaceholderRenderer = Rc<dyn Fn(usize) -> AnyElement>;

/// Where in the syntax tree an offset sits, for editing decisions.
///
/// Parser-independent: `gpui-component` answers from tree-sitter, apps may
/// answer heuristically. `None` (no provider installed) means `Code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxContext {
    Code,
    String,
    Comment,
}

/// Answers syntax context for editing decisions (pairing, skip, indent).
///
/// Created per editor by the application's [`super::LanguageProvider`].
/// Base never imports a parser; implementations live in UI crates or apps.
pub trait SyntaxContextProvider {
    fn context_at(&self, text: &Rope, offset: usize) -> SyntaxContext;
}

#[derive(Clone, Copy, Default)]
pub struct DiagnosticColors {
    pub error: Hsla,
    pub warning: Hsla,
    pub info: Hsla,
    pub hint: Hsla,
}

/// Application-owned colors and highlight resolver consumed by editor painting.
#[derive(Clone)]
pub struct InputEditorStyle {
    pub foreground: Hsla,
    pub muted_foreground: Hsla,
    pub background: Hsla,
    pub border: Hsla,
    pub selection: Hsla,
    pub caret: Hsla,
    pub diagnostics: DiagnosticColors,
    pub highlight_styles: SharedHighlightStyleResolver,
    pub editor_invisible: Option<Hsla>,
    pub editor_active_line: Option<Hsla>,
    pub editor_gutter_background: Option<Hsla>,
    pub fold_icon_renderer: Option<FoldIconRenderer>,
}

impl InputEditorStyle {
    /// Fills in every colour that was left unset, from the active palette.
    ///
    /// `Hsla::default()` is fully transparent, and every colour on `Default` is
    /// that — so an input nothing projected onto painted its glyphs, its caret
    /// and its selection in nothing at all. Transparent is not a colour anyone
    /// means for ink, which is what makes it usable as "unset" here.
    ///
    /// This is resolution, not assignment: whatever a consumer did project is
    /// kept exactly. `crates/component` projects the whole style on every render and
    /// never reaches this; a consumer that projects once at construction gets
    /// the palette that is current now rather than the one that happened to be
    /// installed when the state was built.
    pub fn resolved(&self, tokens: &SemanticThemeTokens) -> Self {
        let colors = &tokens.colors;
        let unset = |value: Hsla| value.a == 0.;
        let or = |value: Hsla, fallback: Hsla| if unset(value) { fallback } else { value };

        let foreground = or(self.foreground, colors.foreground);
        let mut selection = self.selection;
        if unset(selection) {
            selection = colors.accent;
            // A selection must not hide the glyphs it selects.
            selection.a = 0.4;
        }

        Self {
            foreground,
            muted_foreground: or(self.muted_foreground, colors.muted_foreground),
            background: or(self.background, colors.surface),
            border: or(self.border, colors.border),
            selection,
            caret: or(self.caret, foreground),
            ..self.clone()
        }
    }
}

impl Default for InputEditorStyle {
    fn default() -> Self {
        Self {
            foreground: Hsla::default(),
            muted_foreground: Hsla::default(),
            background: Hsla::default(),
            border: Hsla::default(),
            selection: Hsla::default(),
            caret: Hsla::default(),
            diagnostics: DiagnosticColors::default(),
            highlight_styles: Arc::new(NoHighlightStyles),
            editor_invisible: None,
            editor_active_line: None,
            editor_gutter_background: None,
            fold_icon_renderer: None,
        }
    }
}
