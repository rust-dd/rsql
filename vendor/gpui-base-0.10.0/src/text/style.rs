use std::sync::Arc;

use gpui::{HighlightStyle, Hsla, Rems, StyleRefinement, rems};

use crate::ColorTokens;

/// TextViewStyle used to customize the style for [`super::TextView`].
///
/// The fields are private because this type crosses the `gpui-base` seam:
/// build one with the `with_*` methods and read it back through the accessors
/// of the same name, so a later field is an additive change rather than a
/// breaking one.
#[derive(Clone)]
pub struct TextViewStyle {
    foreground: Hsla,
    muted_foreground: Hsla,
    link: Hsla,
    selection: Hsla,
    code_background: Hsla,
    border: Hsla,
    paragraph_gap: Rems,
    heading: Arc<dyn Fn(u8) -> StyleRefinement + Send + Sync + 'static>,
    code_block: StyleRefinement,
    table: StyleRefinement,
    table_head: StyleRefinement,
    table_cell: StyleRefinement,
    inline_code: HighlightStyle,
    /// Explicit inline code refinements, applied after theme and surface colors.
    inline_code_style: StyleRefinement,
    /// Explicit list marker refinements, applied after theme and surface colors.
    list_style: StyleRefinement,
    /// Explicit blockquote refinements, applied after theme and surface colors.
    blockquote_style: StyleRefinement,
    /// The table body background, or the theme surface when `None`. Only
    /// [`Self::on_text_color`] sets it, to let a table on an inverted surface
    /// show that surface.
    table_background: Option<Hsla>,
    /// The background of a hovered table body row, or no hover highlight
    /// when `None`.
    table_row_hover: Option<Hsla>,
    is_dark: bool,
}

impl PartialEq for TextViewStyle {
    fn eq(&self, other: &Self) -> bool {
        self.paragraph_gap == other.paragraph_gap
            && self.foreground == other.foreground
            && self.muted_foreground == other.muted_foreground
            && self.link == other.link
            && self.selection == other.selection
            && self.code_background == other.code_background
            && self.border == other.border
            && (Arc::ptr_eq(&self.heading, &other.heading)
                || (1..=6).all(|level| (self.heading)(level) == (other.heading)(level)))
            && self.code_block == other.code_block
            && self.table == other.table
            && self.table_head == other.table_head
            && self.table_cell == other.table_cell
            && self.inline_code == other.inline_code
            && self.inline_code_style == other.inline_code_style
            && self.list_style == other.list_style
            && self.blockquote_style == other.blockquote_style
            && self.table_background == other.table_background
            && self.table_row_hover == other.table_row_hover
            && self.is_dark == other.is_dark
    }
}

impl Default for TextViewStyle {
    fn default() -> Self {
        Self::from_colors(&ColorTokens::light(), false)
    }
}

impl TextViewStyle {
    /// Derives rich-text colors from Base semantic theme tokens.
    pub fn from_theme(theme: &crate::Theme) -> Self {
        Self::from_colors(
            &theme.tokens.colors,
            theme.appearance == crate::ThemeAppearance::Dark,
        )
    }

    /// Derives rich-text colors from one palette.
    ///
    /// Rich text needs a handful of roles the palette does not name directly —
    /// a code background, a link color — so they are mapped here once instead
    /// of at every call site.
    fn from_colors(colors: &ColorTokens, is_dark: bool) -> Self {
        Self {
            foreground: colors.foreground,
            muted_foreground: colors.muted_foreground,
            link: colors.primary,
            selection: colors.selection,
            code_background: colors.accent,
            border: colors.border,
            paragraph_gap: rems(0.75),
            heading: Arc::new(|_| StyleRefinement::default()),
            code_block: StyleRefinement::default(),
            table: StyleRefinement::default(),
            table_head: StyleRefinement::default(),
            table_cell: StyleRefinement::default(),
            inline_code: HighlightStyle {
                background_color: Some(colors.accent),
                ..Default::default()
            },
            inline_code_style: StyleRefinement::default(),
            list_style: StyleRefinement::default(),
            blockquote_style: StyleRefinement::default(),
            table_background: None,
            table_row_hover: None,
            is_dark,
        }
    }

    /// Sets the default body-text color.
    pub fn with_foreground(mut self, color: Hsla) -> Self {
        self.foreground = color;
        self
    }

    /// Sets the secondary text color.
    pub fn with_muted_foreground(mut self, color: Hsla) -> Self {
        self.muted_foreground = color;
        self
    }

    /// Sets the link text color.
    pub fn with_link(mut self, color: Hsla) -> Self {
        self.link = color;
        self
    }

    /// Sets the background painted behind selected text.
    ///
    /// Selection quads are painted under the glyphs, so this is normally a
    /// translucent wash rather than a solid fill.
    pub fn with_selection(mut self, color: Hsla) -> Self {
        self.selection = color;
        self
    }

    /// Sets the background of fenced code blocks and table header rows.
    pub fn with_code_background(mut self, color: Hsla) -> Self {
        self.code_background = color;
        self
    }

    /// Sets the color of borders and horizontal rules.
    pub fn with_border(mut self, color: Hsla) -> Self {
        self.border = color;
        self
    }

    /// Sets the gap between paragraphs. Defaults to 0.75 rem.
    pub fn with_paragraph_gap(mut self, gap: Rems) -> Self {
        self.paragraph_gap = gap;
        self
    }

    /// Sets the style refinement for headings, selected by heading level (1-6).
    pub fn with_heading<F>(mut self, heading: F) -> Self
    where
        F: Fn(u8) -> StyleRefinement + Send + Sync + 'static,
    {
        self.heading = Arc::new(heading);
        self
    }

    /// Sets the style refinement for code blocks.
    ///
    /// Set `overflow.y` to `Overflow::Scroll` together with a max height to
    /// scroll long code inside the block: it gets its own scrollbar, and wheel
    /// input over it no longer scrolls an ancestor list until the code reaches
    /// its edge.
    pub fn with_code_block(mut self, style: StyleRefinement) -> Self {
        self.code_block = style;
        self
    }

    /// Sets the highlight style for inline code spans.
    ///
    /// When `background_color` is `None`, the neutral code background is used,
    /// which keeps [`TextViewStyle::default`] usable without a theme.
    pub fn with_inline_code(mut self, style: HighlightStyle) -> Self {
        self.inline_code = style;
        self
    }

    /// Overrides inline code typography and its box presentation.
    ///
    /// Supports font family, size, features, fallbacks, weight, style, color, underline and
    /// strikethrough, background, border widths/color/style, corner radii, and
    /// padding. Sizes and padding resolve pixels/rem against the window.
    /// Unspecified properties keep the existing defaults. Box layout, margin,
    /// positioning, overflow, and interaction properties are not used.
    /// Line height remains owned by the paragraph. Non-finite or non-positive
    /// font sizes are ignored. Explicit refinements also take precedence on
    /// inverted surfaces.
    pub fn with_inline_code_style(mut self, style: StyleRefinement) -> Self {
        self.inline_code_style = normalize_inline_code_style(style);
        self
    }

    /// Refines the marker of a bullet or numbered list item.
    ///
    /// Applied to the marker column after its defaults: the muted text color,
    /// a minimum width of the list indent, right-aligned text, and a right
    /// padding of 0.3 times that indent. Text properties such as color, font
    /// family, size, and weight style the bullet or number. The text color
    /// also outlines an unchecked task-list checkbox; a checked one keeps the
    /// link color. Unspecified properties keep the existing defaults, and
    /// explicit refinements also take precedence on inverted surfaces.
    pub fn with_list_style(mut self, style: StyleRefinement) -> Self {
        self.list_style = style;
        self
    }

    /// Refines the blockquote container.
    ///
    /// Applied after its defaults: the muted text color, a 2px left border in
    /// the border color, and a left padding of 0.9 times the body font size.
    /// Border widths and color style the quote bar; text properties,
    /// background, padding, and corner radii style the quote and its
    /// contents. Child blocks keep properties they set explicitly, including
    /// heading and table font sizes. Unspecified properties keep the existing
    /// defaults, and explicit refinements also take precedence on inverted surfaces.
    pub fn with_blockquote_style(mut self, style: StyleRefinement) -> Self {
        self.blockquote_style = style;
        self
    }

    /// Sets the style refinement for the table container (the bordered wrapper
    /// in wrap mode, the scroll viewport in horizontal-scroll mode).
    ///
    /// Set `overflow_x: scroll` on the refinement for adaptive table layout:
    /// columns fit their content when space allows, shrink (wrapping cell
    /// text) down to a per-column floor when the frame is narrower, and below
    /// that the table scrolls horizontally instead of squeezing further.
    pub fn with_table(mut self, style: StyleRefinement) -> Self {
        self.table = style;
        self
    }

    /// Sets the style refinement for the header row (the first row) of a
    /// table, applied on top of the header background and foreground.
    pub fn with_table_head(mut self, style: StyleRefinement) -> Self {
        self.table_head = style;
        self
    }

    /// Sets the style refinement for each table cell.
    ///
    /// With the scroll table layout, `white_space: nowrap` here keeps cells on
    /// a single line — columns then never shrink and the table scrolls as soon
    /// as the content is wider than the frame.
    pub fn with_table_cell(mut self, style: StyleRefinement) -> Self {
        self.table_cell = style;
        self
    }

    /// Sets the background of a table body row while the pointer is over it.
    ///
    /// The header row does not highlight. Without this, table rows have no
    /// hover highlight.
    pub fn with_table_row_hover(mut self, color: Hsla) -> Self {
        self.table_row_hover = Some(color);
        self
    }

    /// Sets whether content-specific assets should use their dark variant.
    pub fn with_dark(mut self, is_dark: bool) -> Self {
        self.is_dark = is_dark;
        self
    }

    /// The default body-text color.
    pub fn foreground(&self) -> Hsla {
        self.foreground
    }

    /// The secondary text color.
    pub fn muted_foreground(&self) -> Hsla {
        self.muted_foreground
    }

    /// The link text color.
    pub fn link(&self) -> Hsla {
        self.link
    }

    /// The background painted behind selected text.
    pub fn selection(&self) -> Hsla {
        self.selection
    }

    /// The background of fenced code blocks and table header rows.
    pub fn code_background(&self) -> Hsla {
        self.code_background
    }

    /// The color of borders and horizontal rules.
    pub fn border(&self) -> Hsla {
        self.border
    }

    /// The gap between paragraphs.
    pub fn paragraph_gap(&self) -> Rems {
        self.paragraph_gap
    }

    /// The style refinement for a heading at `level` (1-6).
    pub fn heading(&self, level: u8) -> StyleRefinement {
        (self.heading)(level)
    }

    /// The style refinement for code blocks.
    pub fn code_block(&self) -> &StyleRefinement {
        &self.code_block
    }

    /// The style refinement for the table container.
    pub fn table(&self) -> &StyleRefinement {
        &self.table
    }

    /// The style refinement for table header rows.
    pub fn table_head(&self) -> &StyleRefinement {
        &self.table_head
    }

    /// The style refinement for table cells.
    pub fn table_cell(&self) -> &StyleRefinement {
        &self.table_cell
    }

    /// The background of a hovered table body row, if rows highlight on hover.
    pub fn table_row_hover(&self) -> Option<Hsla> {
        self.table_row_hover
    }

    /// The highlight style for inline code, before the code-background
    /// fallback in [`Self::inline_code_highlight`] applies.
    pub fn inline_code(&self) -> HighlightStyle {
        self.inline_code
    }

    /// Explicit inline code typography and box refinements.
    pub fn inline_code_style(&self) -> &StyleRefinement {
        &self.inline_code_style
    }

    /// Explicit list marker refinements.
    pub fn list_style(&self) -> &StyleRefinement {
        &self.list_style
    }

    /// Explicit blockquote refinements.
    pub fn blockquote_style(&self) -> &StyleRefinement {
        &self.blockquote_style
    }

    /// Whether content-specific assets should use their dark variant.
    pub fn is_dark(&self) -> bool {
        self.is_dark
    }

    /// The table body background, when it is not the theme surface.
    pub(crate) fn table_background(&self) -> Option<Hsla> {
        self.table_background
    }

    /// Returns the [`HighlightStyle`] to use for inline code, falling back to
    /// the code background when no custom background was supplied.
    pub(crate) fn inline_code_highlight(&self) -> HighlightStyle {
        let mut style = self.inline_code;
        if style.background_color.is_none() {
            style.background_color = Some(self.code_background);
        }
        style
    }

    /// This style adapted to body text drawn in `color`, the text color a
    /// container sets for its surface.
    ///
    /// The body text always takes `color`. When `color` is far from this
    /// style's foreground in lightness, the surface is inverted from the one
    /// this style was made for (a `primary` fill, say): the link, muted text,
    /// code, border, selection and table row hover colors would vanish on it,
    /// so they are all derived from `color`, and [`Self::is_dark`] flips.
    pub(crate) fn on_text_color(&self, color: Hsla) -> Self {
        let style = self.clone().with_foreground(color);
        if !self.is_inverted_by(color) {
            return style;
        }

        let code_background = color.opacity(0.12);
        let mut table_head = self.table_head.clone();
        table_head.background = Some(code_background.into());
        table_head.text.color = Some(color);
        let mut style = style
            .with_muted_foreground(color.opacity(0.7))
            .with_link(color)
            .with_selection(color.opacity(0.25))
            .with_code_background(code_background)
            .with_border(color.opacity(0.2))
            .with_inline_code(HighlightStyle {
                background_color: Some(code_background),
                ..self.inline_code
            })
            .with_table_head(table_head)
            .with_dark(!self.is_dark);
        style.table_background = Some(gpui::transparent_black());
        style.table_row_hover = self.table_row_hover.map(|_| color.opacity(0.08));
        style
    }

    /// Whether body text in `color` sits on a surface inverted from the one
    /// this style was made for.
    ///
    /// Mid-tone text such as a destructive red reads on either kind of surface,
    /// so only a lightness gap wider than that counts as inverted.
    pub(crate) fn is_inverted_by(&self, color: Hsla) -> bool {
        const INVERTED_LIGHTNESS_GAP: f32 = 0.6;
        (oklab_lightness(color) - oklab_lightness(self.foreground)).abs() > INVERTED_LIGHTNESS_GAP
    }
}

/// The perceptual (Oklab) lightness of `color`, from 0 (black) to 1 (white).
fn oklab_lightness(color: Hsla) -> f32 {
    let rgb = color.to_rgb();
    let linear = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (linear(rgb.r), linear(rgb.g), linear(rgb.b));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s
}

/// Drop an invalid font size before it can replace an inherited value or
/// make retained style equality fail every frame (NaN is never equal).
pub(super) fn normalize_inline_code_style(mut style: StyleRefinement) -> StyleRefinement {
    if style.text.font_size.is_some_and(|size| {
        let size = size.to_pixels(gpui::px(1.)).as_f32();
        !size.is_finite() || size <= 0.
    }) {
        style.text.font_size = None;
    }
    style
}
