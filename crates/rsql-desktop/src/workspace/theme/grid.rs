use super::*;

pub(crate) struct GridColors {
    pub text: Hsla,
    pub null: Hsla,
    pub fk: Hsla,
    pub modified: Hsla,
    pub deleted: Hsla,
    pub deleted_text: Hsla,
    pub loading: Hsla,
}

fn hsl(h: f32, s: f32, l: f32) -> Hsla {
    hsla(h / 360., s / 100., l / 100., 1.)
}

pub(crate) fn colors(dark: bool) -> GridColors {
    GridColors {
        text: if dark {
            hsl(250., 15., 85.)
        } else {
            rgb(0x1a1a2e).into()
        },
        null: hsl(250., 10., 50.),
        fk: hsl(220., 70., 50.),
        modified: Hsla::from(rgb(0xf59e0b)).opacity(if dark { 0.15 } else { 0.1 }),
        deleted: Hsla::from(rgb(0xef4444)).opacity(0.1),
        deleted_text: rgb(0x999999).into(),
        loading: rgb(0x888888).into(),
    }
}

pub(super) fn apply(theme: &mut Theme, dark: bool) {
    let role = |light, night| if dark { night } else { Hsla::from(rgb(light)) };
    theme.table = role(0xffffff, hsl(250., 15., 13.));
    theme.table_head = role(0xf5f5f8, hsl(250., 15., 18.));
    theme.table_even = role(0xfafafa, hsl(250., 15., 15.));
    theme.table_head_foreground = role(0x333340, hsl(250., 15., 75.));
    theme.table_hover = role(0xf0f0f4, hsl(250., 15., 20.));
    theme.table_row_border = role(0xe2e2e8, hsl(250., 12., 22.));
    theme.table_active_border = hsl(260., 70., if dark { 60. } else { 55. });
    theme.table_active = theme
        .table_active_border
        .opacity(if dark { 0.15 } else { 0.1 });
    theme.tokens.table = theme.table.into();
    theme.tokens.table_even = theme.table_even.into();
    theme.tokens.table_active = theme.table_active.into();
    theme.tokens.table_hover = theme.table_hover.into();
    theme.tokens.table_head = theme.table_head.into();
}
