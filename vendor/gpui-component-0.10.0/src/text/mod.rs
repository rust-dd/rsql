//! Compatibility facade for rich text now owned by `gpui-base`.

mod compat;
mod frontmatter;
mod style;

pub use compat::{
    Text, TextView, TextViewLayoutState, TextViewPlugin, TextViewPrepaintState, html, markdown,
};
pub use frontmatter::FrontmatterPlugin;
pub use gpui_base::text::{
    InlineElement, InlineRenderContext, MarkdownBlockParserFn, MarkdownBlockRenderFn,
    MarkdownExtensions, MarkdownNode, MarkdownParseContext, MarkdownPlugin, RangeHighlight,
    RangeHighlightError, RenderedText, SelectionFormat, TableData, TaskToggleEvent, TextViewMotion,
    TextViewState, markdown_ast,
};
pub use style::TextViewStyle;

#[cfg(any(feature = "tree-sitter", feature = "syntect"))]
use std::cell::RefCell;
#[cfg(feature = "tree-sitter")]
use std::collections::HashMap;

use gpui::Styled as _;
#[cfg(feature = "tree-sitter")]
use gpui_base::input::{InputEdit, Point, RopeExt as _};
#[cfg(feature = "tree-sitter")]
use ropey::Rope;

#[cfg(feature = "tree-sitter")]
use crate::highlighter::{LanguageRegistry, SyntaxHighlighter};

/// Derives the Base rich-text style installed by the component theme adapter.
pub(crate) fn base_text_view_style(theme: &crate::Theme) -> gpui_base::TextViewStyle {
    let radius = theme.semantic_tokens().radius.md;
    let mut table = gpui::StyleRefinement::default();
    table.corner_radii.top_left = Some(radius.into());
    table.corner_radii.top_right = Some(radius.into());
    table.corner_radii.bottom_left = Some(radius.into());
    table.corner_radii.bottom_right = Some(radius.into());
    let mut code_block = gpui::StyleRefinement::default();
    code_block.corner_radii = table.corner_radii.clone();
    let table_head = gpui::StyleRefinement::default()
        .bg(theme.table_head)
        .text_color(theme.table_head_foreground);

    gpui_base::TextViewStyle::default()
        .with_foreground(theme.foreground)
        .with_muted_foreground(theme.muted_foreground)
        .with_link(theme.link)
        .with_selection(theme.selection)
        .with_code_background(theme.muted)
        .with_border(theme.border)
        .with_code_block(code_block)
        .with_table(table)
        .with_table_head(table_head)
        .with_table_row_hover(theme.table_hover)
        .with_inline_code(gpui::HighlightStyle {
            background_color: Some(theme.accent),
            ..Default::default()
        })
        .with_dark(theme.is_dark())
}

pub(crate) fn install_text_view_defaults(theme: &crate::Theme, cx: &mut gpui::App) {
    // Component's Root sets the theme foreground, so every container that
    // sets its own text color is one a text view should follow.
    let defaults = gpui_base::TextViewDefaults::new()
        .with_style(base_text_view_style(theme))
        .with_inherit_text_color(true);

    #[cfg(any(feature = "tree-sitter", feature = "syntect"))]
    let defaults = defaults.with_code_block_highlighter(component_code_block_highlighter(
        theme.highlight_theme.clone(),
    ));

    defaults.install(cx);
}

#[cfg(feature = "tree-sitter")]
pub(crate) fn component_code_block_highlighter(
    highlight_theme: std::sync::Arc<crate::highlighter::HighlightTheme>,
) -> impl Fn(&gpui_base::text::CodeBlock) -> Vec<(std::ops::Range<usize>, gpui::HighlightStyle)>
+ Send
+ Sync
+ 'static {
    move |block| {
        thread_local! {
            static HIGHLIGHTERS: RefCell<HashMap<gpui::SharedString, SyntaxHighlighter>> =
                RefCell::new(HashMap::new());
        }

        let Some(lang) = block.lang() else {
            return Vec::new();
        };
        let code = block.code();
        HIGHLIGHTERS.with(|cache| {
            let mut cache = cache.borrow_mut();
            let highlighter = cache
                .entry(lang.clone())
                .or_insert_with(|| SyntaxHighlighter::new(lang.as_ref()));
            if let Some(config) = LanguageRegistry::singleton().language(lang.as_ref())
                && highlighter.language() != &config.name
            {
                *highlighter = SyntaxHighlighter::new(lang.as_ref());
            }

            let old_end_byte = highlighter.text().len();
            let old_end_position = highlighter.text().offset_to_point(old_end_byte);
            let code_rope = Rope::from_str(code.as_ref());
            let edit = InputEdit {
                start_byte: 0,
                old_end_byte,
                new_end_byte: code.len(),
                start_position: Point::new(0, 0),
                old_end_position,
                new_end_position: code_rope.offset_to_point(code.len()),
            };
            highlighter.update_input(Some(edit), &code_rope, None);
            highlighter.styles(&(0..code.len()), highlight_theme.as_ref())
        })
    }
}

/// The syntect counterpart: syntect keeps no state between calls, and the
/// highlights of an unchanged block are cached by the text view itself.
#[cfg(all(feature = "syntect", not(feature = "tree-sitter")))]
pub(crate) fn component_code_block_highlighter(
    highlight_theme: std::sync::Arc<crate::highlighter::HighlightTheme>,
) -> impl Fn(&gpui_base::text::CodeBlock) -> Vec<(std::ops::Range<usize>, gpui::HighlightStyle)>
+ Send
+ Sync
+ 'static {
    move |block| match block.lang() {
        Some(lang) => crate::highlighter::highlight_code(
            lang.as_ref(),
            block.code().as_ref(),
            highlight_theme.as_ref(),
        ),
        None => Vec::new(),
    }
}

/// The type [`shared_code_block_highlighter`] returns.
#[cfg(any(feature = "tree-sitter", feature = "syntect"))]
pub(crate) type SharedCodeBlockHighlighter = dyn Fn(&gpui_base::text::CodeBlock) -> Vec<(std::ops::Range<usize>, gpui::HighlightStyle)>
    + Send
    + Sync;

/// The [`component_code_block_highlighter`] for `highlight_theme`, built once
/// and handed out again for the same theme.
///
/// A code block reuses its highlights only while the highlighter is the same
/// `Arc`, and a text view with a custom theme is laid out every frame, so a
/// fresh highlighter per frame reparsed every code block with tree-sitter on
/// every frame. Each entry keeps its theme alive, so only the most recent few
/// themes are kept.
///
/// Entries are also keyed on the [`LanguageRegistry`] generation: registering
/// a language drops them, so the next frame gets a new highlighter and code
/// blocks painted before the language existed are highlighted again.
#[cfg(any(feature = "tree-sitter", feature = "syntect"))]
pub(crate) fn shared_code_block_highlighter(
    highlight_theme: &std::sync::Arc<crate::highlighter::HighlightTheme>,
) -> std::sync::Arc<SharedCodeBlockHighlighter> {
    // Syntect's languages are fixed at build time, so its entries never go stale.
    #[cfg(feature = "tree-sitter")]
    let generation = LanguageRegistry::singleton().generation();
    #[cfg(not(feature = "tree-sitter"))]
    let generation = 0;
    shared_code_block_highlighter_at(highlight_theme, generation)
}

/// [`shared_code_block_highlighter`] at an explicit registry `generation`.
#[cfg(any(feature = "tree-sitter", feature = "syntect"))]
fn shared_code_block_highlighter_at(
    highlight_theme: &std::sync::Arc<crate::highlighter::HighlightTheme>,
    generation: u64,
) -> std::sync::Arc<SharedCodeBlockHighlighter> {
    use std::sync::Arc;

    use crate::highlighter::HighlightTheme;

    /// The registry generation the entries were built at, and the entries.
    type SharedHighlighters = (
        u64,
        Vec<(Arc<HighlightTheme>, Arc<SharedCodeBlockHighlighter>)>,
    );

    const CAPACITY: usize = 4;
    thread_local! {
        static SHARED: RefCell<SharedHighlighters> = const { RefCell::new((0, Vec::new())) };
    }

    SHARED.with(|cache| {
        let (cached_generation, shared) = &mut *cache.borrow_mut();
        if *cached_generation != generation {
            *cached_generation = generation;
            shared.clear();
        }
        if let Some((_, highlighter)) = shared
            .iter()
            .find(|(theme, _)| Arc::ptr_eq(theme, highlight_theme))
        {
            return highlighter.clone();
        }

        let highlighter: Arc<SharedCodeBlockHighlighter> =
            Arc::new(component_code_block_highlighter(highlight_theme.clone()));
        if shared.len() == CAPACITY {
            shared.remove(0);
        }
        shared.push((highlight_theme.clone(), highlighter.clone()));
        highlighter
    })
}
