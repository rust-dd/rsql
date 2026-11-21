use gpui::{
    App, Hsla, ImageSource, InteractiveElement, Interactivity, IntoElement, ParentElement as _,
    RenderOnce, SharedString, StyleRefinement, Styled, Window, div, prelude::FluentBuilder,
};
use gpui_base::{Avatar as BaseAvatar, AvatarFallback, AvatarImage, TestTextExt as _};

use crate::{
    ActiveTheme, Icon, IconName, Sizable, Size, StyledExt, ThemeStyled as _,
    avatar::{AvatarSized as _, avatar_size},
    oklch,
};

/// User avatar element.
///
/// We can use [`Sizable`] trait to set the size of the avatar (see also: [`avatar_size`] about the size in pixels).
#[derive(IntoElement)]
pub struct Avatar {
    base: BaseAvatar,
    style: StyleRefinement,
    src: Option<ImageSource>,
    name: Option<SharedString>,
    short_name: SharedString,
    placeholder: Icon,
    size: Size,
}

impl Avatar {
    pub fn new() -> Self {
        Self {
            base: BaseAvatar::new(),
            style: StyleRefinement::default(),
            src: None,
            name: None,
            short_name: SharedString::default(),
            placeholder: Icon::new(IconName::User),
            size: Size::Medium,
        }
    }

    /// Set to use image source for the avatar.
    pub fn src(mut self, source: impl Into<ImageSource>) -> Self {
        self.src = Some(source.into());
        self
    }

    /// Set name of the avatar user, if `src` is none, will use this name as placeholder.
    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        let name: SharedString = name.into();
        let short: SharedString = extract_text_initials(&name).into();

        self.name = Some(name);
        self.short_name = short;
        self
    }

    /// Set placeholder icon, default: [`IconName::User`]
    pub fn placeholder(mut self, icon: impl Into<Icon>) -> Self {
        self.placeholder = icon.into();
        self
    }
}

impl Sizable for Avatar {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl Styled for Avatar {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl InteractiveElement for Avatar {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for Avatar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let corner_radii = self.style.corner_radii.clone();
        let mut inner_style = StyleRefinement::default();
        inner_style.corner_radii = corner_radii;

        let identity = self
            .name
            .is_some()
            .then(|| IdentityColor::new(&self.short_name, cx));

        // The tinted outline belongs to the initials. An image, or the anonymous
        // placeholder, keeps the neutral border.
        let border_color = match (identity, &self.src) {
            (Some(identity), None) => identity.border,
            _ => cx.theme().border,
        };

        let fallback = AvatarFallback::new()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full_style(cx)
            .overflow_hidden()
            .when_none(&identity, |this| {
                this.text_size(avatar_size(self.size) * 0.6)
                    .child(self.placeholder)
            })
            .when_some(identity, |this, identity| {
                this.bg(identity.background)
                    .text_color(identity.foreground)
                    .child(div().avatar_text_size(self.size).child(self.short_name))
            })
            .refine_style(&inner_style);

        // The name is the avatar's alternative text, as an `<img alt>` is.
        let name = self.name.clone();
        self.base
            .size(avatar_size(self.size))
            .flex_shrink_0()
            .rounded_full_style(cx)
            .overflow_hidden()
            .bg(cx.theme().tokens.secondary)
            .text_color(cx.theme().background)
            .border_1()
            .border_color(border_color)
            .fallback(fallback)
            .when_some(self.src, |this, src| {
                this.image(
                    AvatarImage::new(src)
                        .size_full()
                        .rounded_full_style(cx)
                        .refine_style(&inner_style),
                )
            })
            .refine_style(&self.style)
            .map(|this| match name {
                Some(name) => this.test_alt_text(name).into_any_element(),
                None => this.into_any_element(),
            })
    }
}

/// The colors a name-based fallback draws itself in, picked from the initials so
/// the same person always gets the same ones.
///
/// The ring is 12 evenly spaced OkLCH hues at a fixed lightness and chroma.
/// Unlike an HSL rotation, which changes perceived brightness as it turns, that
/// keeps every avatar at one visual weight and its text legible on every hue.
#[derive(Debug, Clone, Copy, PartialEq)]
struct IdentityColor {
    background: Hsla,
    foreground: Hsla,
    border: Hsla,
}

impl IdentityColor {
    const HUES: u64 = 12;
    const HUE_STEP: f32 = 360. / Self::HUES as f32;

    fn new(short_name: &SharedString, cx: &App) -> Self {
        let hue = (gpui::hash(short_name) % Self::HUES) as f32 * Self::HUE_STEP;
        Self::from_hue(hue, cx.theme().is_dark())
    }

    fn from_hue(hue: f32, is_dark: bool) -> Self {
        // Background and foreground hold WCAG AA against each other, and the
        // border carries the most chroma sRGB has at its lightness for every
        // hue. Both are pinned by the tests below.
        let (background, foreground, border) = if is_dark {
            (
                oklch(0.30, 0.05, hue),
                oklch(0.82, 0.11, hue),
                oklch(0.36, 0.06, hue),
            )
        } else {
            (
                oklch(0.97, 0.032, hue),
                oklch(0.50, 0.145, hue),
                oklch(0.89, 0.05, hue),
            )
        };

        Self {
            background,
            foreground,
            border,
        }
    }
}

fn extract_text_initials(text: &str) -> String {
    let mut result = text
        .split(" ")
        .flat_map(|word| word.chars().next().map(|c| c.to_string()))
        .take(2)
        .collect::<Vec<String>>()
        .join("");

    if result.len() == 1 {
        result = text.chars().take(2).collect::<String>();
    }

    result.to_uppercase()
}
