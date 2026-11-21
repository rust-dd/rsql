use std::ops::Range;

use anyhow::Result;
use gpui::{App, Context, HighlightStyle, SharedString, Task, Window};
use instant::Duration;
use lsp_types::{Position, SemanticTokens, SemanticTokensLegend};
use ropey::Rope;

use crate::input::{EditorMode, HighlightStyleResolver, InputBaseState, Lsp, RopeExt};

/// A provider of semantic highlighting tokens, layered on top of the
/// built-in tree-sitter [`SyntaxHighlighter`](crate::highlighter::SyntaxHighlighter).
///
/// This is the editor counterpart of the LSP
/// `textDocument/semanticTokens/range` request (and Monaco Editor's
/// [`DocumentRangeSemanticTokensProvider`][monaco]). Like the other
/// providers on [`Lsp`](crate::input::Lsp) — `DocumentColorProvider`,
/// `HoverProvider`, … — it is installed on `InputBaseState::lsp`, fetched
/// asynchronously when the document changes, and its result is cached and
/// composed into the render pipeline. It does **not** replace the
/// tree-sitter highlighter.
///
/// # Token names and theming
///
/// Returned tokens are delta-encoded with a numeric `token_type` that
/// indexes [`legend`](Self::legend)`.token_types`. The editor resolves each
/// type *name* against the active
/// [`HighlightTheme`](crate::highlighter::HighlightTheme) at paint time —
/// the same vocabulary the tree-sitter path uses (`"keyword"`, `"comment"`,
/// `"string"`, …; `"keyword.modifier"` falls back to `"keyword"`). Because
/// the color is resolved from the name on every paint, theme switches
/// recolor semantic tokens with no provider cooperation. Token *modifiers*
/// are accepted but not currently mapped to styles.
///
/// [monaco]: https://microsoft.github.io/monaco-editor/
pub trait DocumentRangeSemanticTokensProvider {
    /// The legend naming the numeric `token_type` field of the tokens
    /// returned by [`semantic_tokens`](Self::semantic_tokens). Each entry in
    /// [`SemanticTokensLegend::token_types`] is resolved against the active
    /// [`HighlightTheme`](crate::highlighter::HighlightTheme).
    fn legend(&self) -> SemanticTokensLegend;

    /// Fetches semantic tokens for the specified byte range.
    ///
    /// textDocument/semanticTokens/range
    ///
    /// <https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_semanticTokens>
    fn semantic_tokens(
        &self,
        text: &Rope,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<SemanticTokens>>;
}

impl Lsp {
    /// Get semantic token styles that intersect with the visible byte range,
    /// resolving each cached token's type name against `theme`.
    ///
    /// Called on every paint. The cache is sorted by start position, so this
    /// binary-searches the small window of tokens that can touch the viewport
    /// (`O(log N + visible)`) instead of scanning the whole document — only
    /// the windowed candidates pay the position→byte conversion. Tokens
    /// resolving to an empty byte range, or whose type name the theme does
    /// not recognize, are skipped.
    ///
    /// Returns byte ranges and styles.
    pub(crate) fn semantic_tokens_for_range(
        &self,
        text: &Rope,
        visible_range: &Range<usize>,
        theme: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        if self.semantic_tokens.is_empty() {
            return Vec::new();
        }

        let visible_start = text.offset_to_position(visible_range.start);
        let visible_end = text.offset_to_position(visible_range.end);

        // Cache is sorted by `range.start`. A token can only touch the
        // viewport if its start is before `visible_end` (upper bound) and it
        // is not on a line entirely above the viewport's first line (lower
        // bound — tokens are single-line, so an earlier line cannot reach in).
        let hi = self
            .semantic_tokens
            .partition_point(|(range, _)| range.start < visible_end);
        let lo = self
            .semantic_tokens
            .partition_point(|(range, _)| range.start.line < visible_start.line);

        self.semantic_tokens[lo..hi]
            .iter()
            .filter_map(|(range, name)| {
                let start = text.position_to_offset(&range.start);
                let end = text.position_to_offset(&range.end);
                if start >= end || start >= visible_range.end || end <= visible_range.start {
                    return None;
                }

                let style = theme.style(name.as_ref())?;
                Some((start..end, style))
            })
            .collect()
    }

    pub(crate) fn update_semantic_tokens(
        &mut self,
        text: &Rope,
        window: &mut Window,
        cx: &mut Context<InputBaseState<EditorMode>>,
    ) {
        let Some(provider) = self.semantic_tokens_provider.as_ref() else {
            return;
        };

        let provider = provider.clone();
        let legend = provider.legend();
        let text = text.clone();
        // Fetch the whole document; results are cached and filtered to the
        // viewport at paint time (mirrors `update_document_colors`), so a
        // scroll never needs a refetch.
        let range = 0..text.len();
        let input_state = cx.entity();

        // debounce timer 100ms
        self._semantic_tokens_task = cx.spawn_in(window, async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;

            let task_result = cx
                .update(|window, cx| provider.semantic_tokens(&text, range, window, cx))
                .ok();

            if let Some(task) = task_result {
                if let Ok(tokens) = task.await {
                    let decoded = decode_semantic_tokens(&tokens, &legend);
                    let _ = input_state.update(cx, |input_state, cx| {
                        if decoded != input_state.extras.lsp.semantic_tokens {
                            input_state.extras.lsp.semantic_tokens = decoded;
                            cx.notify();
                        }
                    });
                }
            }
        });
    }
}

/// Decode the LSP delta-encoding of `tokens` into absolute
/// (position-range, type-name) pairs, sorted by start position.
///
/// The type name is looked up in `legend.token_types`; tokens whose
/// `token_type` index is out of bounds are skipped. Color resolution is
/// deferred to paint time so theme switches take effect without a refetch.
fn decode_semantic_tokens(
    tokens: &SemanticTokens,
    legend: &SemanticTokensLegend,
) -> Vec<(lsp_types::Range, SharedString)> {
    // Resolve the legend names once; tokens then share them via cheap
    // ref-counted clones instead of allocating a String per token.
    let names: Vec<SharedString> = legend
        .token_types
        .iter()
        .map(|t| SharedString::from(t.as_str().to_owned()))
        .collect();

    let mut out = Vec::with_capacity(tokens.data.len());
    let mut line: u32 = 0;
    let mut character: u32 = 0;

    for token in &tokens.data {
        if token.delta_line > 0 {
            line += token.delta_line;
            character = token.delta_start;
        } else {
            character += token.delta_start;
        }

        let Some(name) = names.get(token.token_type as usize) else {
            continue;
        };

        let start = Position::new(line, character);
        let end = Position::new(line, character + token.length);
        out.push((lsp_types::Range { start, end }, name.clone()));
    }

    out.sort_by_key(|(range, _)| range.start);
    out
}
