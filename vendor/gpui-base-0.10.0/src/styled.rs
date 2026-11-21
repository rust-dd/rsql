use gpui::{
    App, BoxShadow, Corners, DefiniteLength, Div, Edges, FocusHandle, Hsla, Pixels,
    Refineable as _, Role, StyleRefinement, Styled, Window, div, hsla, point,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RoleOverride {
    #[default]
    Implicit,
    Presentational,
    Role(Role),
}

impl RoleOverride {
    pub fn resolve(self, default: impl FnOnce() -> Role) -> Option<Role> {
        match self {
            Self::Implicit => Some(default()),
            Self::Presentational => None,
            Self::Role(role) => Some(role),
        }
    }
}
impl From<Role> for RoleOverride {
    fn from(role: Role) -> Self {
        Self::Role(role)
    }
}
impl From<Option<Role>> for RoleOverride {
    fn from(role: Option<Role>) -> Self {
        role.map_or(Self::Presentational, Self::Role)
    }
}

/// A row that centers its children on the cross axis.
///
/// See [`StyledExt::h_flex`] for the cross-axis rule, which is not symmetric
/// with [`v_flex`].
pub fn h_flex() -> Div {
    div().h_flex()
}

/// A column whose children stretch across the cross axis.
///
/// See [`StyledExt::v_flex`] for the cross-axis rule, which is not symmetric
/// with [`h_flex`].
pub fn v_flex() -> Div {
    div().v_flex()
}

pub fn box_shadow(
    x: impl Into<Pixels>,
    y: impl Into<Pixels>,
    blur: impl Into<Pixels>,
    spread: impl Into<Pixels>,
    color: Hsla,
) -> BoxShadow {
    BoxShadow {
        offset: point(x.into(), y.into()),
        blur_radius: blur.into(),
        spread_radius: spread.into(),
        inset: false,
        color,
    }
}

macro_rules! font_weight {
    ($method:ident, $weight:ident) => {
        fn $method(self) -> Self {
            self.font_weight(gpui::FontWeight::$weight)
        }
    };
}

#[cfg_attr(
    any(feature = "inspector", debug_assertions),
    gpui_macros::derive_inspector_reflection
)]
pub trait StyledExt: Styled + Sized {
    fn refine_style(mut self, style: &StyleRefinement) -> Self {
        self.style().refine(style);
        self
    }

    /// Lays children out in a row, centered on the cross axis.
    ///
    /// The centering is the desktop default for a row of controls — an icon
    /// beside its label lines up without either side asking for it — but it is
    /// **not** the mirror image of [`Self::v_flex`], which leaves the cross axis
    /// stretching. A column placed in a row therefore does not take the row's
    /// height: it takes its content's height and is centered inside the row.
    /// When its content is taller than the row, it overflows equally above and
    /// below, so the column's header is pushed off the top edge and clipped.
    ///
    /// Give a full-height column `h_full()` (or the row `items_start()` /
    /// `items_stretch()`) whenever the child owns a header, a footer, or a
    /// scroll region that has to resolve against the row's height.
    ///
    /// ```
    /// use gpui_base::StyledExt as _;
    /// use gpui::{ParentElement as _, Styled as _, div};
    ///
    /// // A sidebar beside a detail pane, both spanning the full height.
    /// div().h_flex().size_full().child(div().w_64().h_full());
    /// ```
    fn h_flex(self) -> Self {
        self.flex().flex_row().items_center()
    }

    /// Lays children out in a column, stretching them across the cross axis.
    ///
    /// Unlike [`Self::h_flex`] this installs no cross-axis alignment, so a child
    /// without a width fills the column. See `h_flex` for the asymmetry.
    fn v_flex(self) -> Self {
        self.flex().flex_col()
    }

    fn paddings<L>(self, paddings: impl Into<Edges<L>>) -> Self
    where
        L: Into<DefiniteLength> + Clone + Default + std::fmt::Debug + PartialEq,
    {
        let paddings = paddings.into();
        self.pt(paddings.top.into())
            .pb(paddings.bottom.into())
            .pl(paddings.left.into())
            .pr(paddings.right.into())
    }

    fn margins<L>(self, margins: impl Into<Edges<L>>) -> Self
    where
        L: Into<DefiniteLength> + Clone + Default + std::fmt::Debug + PartialEq,
    {
        let margins = margins.into();
        self.mt(margins.top.into())
            .mb(margins.bottom.into())
            .ml(margins.left.into())
            .mr(margins.right.into())
    }

    fn debug_red(self) -> Self {
        if cfg!(debug_assertions) {
            self.border_1().border_color(hsl(0., 72.2, 50.6))
        } else {
            self
        }
    }

    fn debug_blue(self) -> Self {
        if cfg!(debug_assertions) {
            self.border_1().border_color(hsl(217.2, 91.2, 59.8))
        } else {
            self
        }
    }

    fn debug_yellow(self) -> Self {
        if cfg!(debug_assertions) {
            self.border_1().border_color(hsl(47.9, 95.8, 53.1))
        } else {
            self
        }
    }

    fn debug_green(self) -> Self {
        if cfg!(debug_assertions) {
            self.border_1().border_color(hsl(142.1, 70.6, 45.3))
        } else {
            self
        }
    }

    fn debug_pink(self) -> Self {
        if cfg!(debug_assertions) {
            self.border_1().border_color(hsl(330.4, 81.2, 60.4))
        } else {
            self
        }
    }

    fn debug_focused(self, focus_handle: &FocusHandle, window: &Window, cx: &App) -> Self {
        if cfg!(debug_assertions) && focus_handle.contains_focused(window, cx) {
            self.debug_blue()
        } else {
            self
        }
    }

    font_weight!(font_thin, THIN);
    font_weight!(font_extralight, EXTRA_LIGHT);
    font_weight!(font_light, LIGHT);
    font_weight!(font_normal, NORMAL);
    font_weight!(font_medium, MEDIUM);
    font_weight!(font_semibold, SEMIBOLD);
    font_weight!(font_bold, BOLD);
    font_weight!(font_extrabold, EXTRA_BOLD);
    font_weight!(font_black, BLACK);

    fn corner_radii(self, radius: Corners<Pixels>) -> Self {
        self.rounded_tl(radius.top_left)
            .rounded_tr(radius.top_right)
            .rounded_bl(radius.bottom_left)
            .rounded_br(radius.bottom_right)
    }
}

impl<E: Styled> StyledExt for E {}

#[cfg(any(feature = "inspector", debug_assertions))]
pub fn styled_ext_reflection_methods<T: Styled + 'static>()
-> Vec<gpui::inspector_reflection::FunctionReflection<T>> {
    styled_ext_reflection::methods::<T>()
}

fn hsl(hue: f32, saturation: f32, lightness: f32) -> Hsla {
    hsla(hue / 360., saturation / 100., lightness / 100., 1.)
}
