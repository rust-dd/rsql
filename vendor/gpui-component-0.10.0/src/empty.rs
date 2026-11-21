use gpui::{
    AnyElement, App, IntoElement, ParentElement, RenderOnce, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder as _, relative, rems,
};

use crate::{ActiveTheme as _, StyledExt as _, v_flex};

/// A presentational empty state with independently styled header and content slots.
///
/// The application decides when this element is shown and owns the actions and
/// state of its children. Additional direct children follow the named slots,
/// regardless of builder call order.
#[derive(IntoElement)]
pub struct Empty {
    style: StyleRefinement,
    header: Option<EmptyHeader>,
    content: Option<EmptyContent>,
    children: Vec<AnyElement>,
}

impl Empty {
    /// Create an empty state without a background or visible border.
    pub fn new() -> Self {
        Self {
            style: StyleRefinement::default(),
            header: None,
            content: None,
            children: Vec::new(),
        }
    }

    /// Set the header, replacing any previously configured header.
    pub fn header(mut self, header: EmptyHeader) -> Self {
        self.header = Some(header);
        self
    }

    /// Set the content, replacing any previously configured content.
    pub fn content(mut self, content: EmptyContent) -> Self {
        self.content = Some(content);
        self
    }
}

impl Default for Empty {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for Empty {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for Empty {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Empty {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .w_full()
            .min_w_0()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_4()
            .p_6()
            .rounded(cx.theme().radius_tokens().xl)
            .border_dashed()
            .border_color(cx.theme().border)
            .text_center()
            .text_color(cx.theme().foreground)
            .refine_style(&self.style)
            .when_some(self.header, |this, header| this.child(header))
            .when_some(self.content, |this, content| this.child(content))
            .children(self.children)
    }
}

/// The media, title, and description of an [`Empty`] state, in that order.
///
/// Each slot is optional. Replacing one slot leaves the other slots intact.
#[derive(IntoElement)]
pub struct EmptyHeader {
    style: StyleRefinement,
    media: Option<EmptyMedia>,
    title: Option<EmptyTitle>,
    description: Option<EmptyDescription>,
}

impl EmptyHeader {
    /// Create a centered header with a maximum width of 24 rem.
    pub fn new() -> Self {
        Self {
            style: StyleRefinement::default(),
            media: None,
            title: None,
            description: None,
        }
    }

    /// Set the media, replacing any previously configured media.
    pub fn media(mut self, media: EmptyMedia) -> Self {
        self.media = Some(media);
        self
    }

    /// Set the title, replacing any previously configured title.
    pub fn title(mut self, title: EmptyTitle) -> Self {
        self.title = Some(title);
        self
    }

    /// Set the description, replacing any previously configured description.
    pub fn description(mut self, description: EmptyDescription) -> Self {
        self.description = Some(description);
        self
    }
}

impl Default for EmptyHeader {
    fn default() -> Self {
        Self::new()
    }
}

impl Styled for EmptyHeader {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for EmptyHeader {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        v_flex()
            .w_full()
            .max_w(rems(24.))
            .min_w_0()
            .items_center()
            .gap_2()
            .refine_style(&self.style)
            .when_some(self.media, |this, media| this.child(media))
            .when_some(self.title, |this, title| this.child(title))
            .when_some(self.description, |this, description| {
                this.child(description)
            })
    }
}

/// Visual treatment for the media of an [`Empty`] state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EmptyMediaVariant {
    /// Unframed content such as an image, avatar, or avatar group.
    #[default]
    Default,
    /// A muted, rounded two-rem frame for an icon.
    Icon,
}

/// A media slot accepting icons, images, avatars, and custom elements.
#[derive(IntoElement)]
pub struct EmptyMedia {
    style: StyleRefinement,
    variant: EmptyMediaVariant,
    children: Vec<AnyElement>,
}

impl EmptyMedia {
    /// Create an unframed media slot.
    pub fn new() -> Self {
        Self {
            style: StyleRefinement::default(),
            variant: EmptyMediaVariant::Default,
            children: Vec::new(),
        }
    }

    /// Set the visual treatment without changing the supplied children.
    pub fn with_variant(mut self, variant: EmptyMediaVariant) -> Self {
        self.variant = variant;
        self
    }
}

impl Default for EmptyMedia {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for EmptyMedia {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for EmptyMedia {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for EmptyMedia {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        // A column preserves the intrinsic width of nested rows such as AvatarGroup.
        v_flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .mb_2()
            .when(self.variant == EmptyMediaVariant::Icon, |this| {
                this.size_8()
                    .rounded(cx.theme().radius_tokens().lg)
                    .bg(cx.theme().muted)
                    .text_color(cx.theme().foreground)
                    // Icon inherits one rem unless it has an explicit size.
                    .text_base()
            })
            .refine_style(&self.style)
            .children(self.children)
    }
}

/// The title of an [`Empty`] state, accepting text or custom children.
#[derive(IntoElement)]
pub struct EmptyTitle {
    style: StyleRefinement,
    children: Vec<AnyElement>,
}

impl EmptyTitle {
    /// Create an empty title.
    pub fn new() -> Self {
        Self {
            style: StyleRefinement::default(),
            children: Vec::new(),
        }
    }
}

impl Default for EmptyTitle {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for EmptyTitle {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for EmptyTitle {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for EmptyTitle {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .max_w_full()
            .min_w_0()
            .text_sm()
            .font_medium()
            .whitespace_normal()
            .refine_style(&self.style)
            .children(self.children)
    }
}

/// Supporting text or rich content for an [`Empty`] state.
#[derive(IntoElement)]
pub struct EmptyDescription {
    style: StyleRefinement,
    children: Vec<AnyElement>,
}

impl EmptyDescription {
    /// Create a muted description that wraps to the available width.
    pub fn new() -> Self {
        Self {
            style: StyleRefinement::default(),
            children: Vec::new(),
        }
    }
}

impl Default for EmptyDescription {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for EmptyDescription {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for EmptyDescription {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for EmptyDescription {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .w_full()
            .min_w_0()
            .text_sm()
            .line_height(relative(1.625))
            .text_color(cx.theme().muted_foreground)
            .whitespace_normal()
            .refine_style(&self.style)
            .children(self.children)
    }
}

/// Actions, inputs, or other application-owned content below an [`EmptyHeader`].
#[derive(IntoElement)]
pub struct EmptyContent {
    style: StyleRefinement,
    children: Vec<AnyElement>,
}

impl EmptyContent {
    /// Create a centered content column with a maximum width of 24 rem.
    pub fn new() -> Self {
        Self {
            style: StyleRefinement::default(),
            children: Vec::new(),
        }
    }
}

impl Default for EmptyContent {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for EmptyContent {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for EmptyContent {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for EmptyContent {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        v_flex()
            .w_full()
            .max_w(rems(24.))
            .min_w_0()
            .items_center()
            .gap_2p5()
            .text_sm()
            .refine_style(&self.style)
            .children(self.children)
    }
}
