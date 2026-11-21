use crate::terminal::{
    CellFlags, Color, NamedColor, Rgb, TerminalCell, TerminalSnapshot, TerminalTheme,
};
use gpui_kit::{Font, FontStyle, FontWeight, Hsla, hsla, rgb};

pub(super) fn color(screen: &TerminalSnapshot, color: Color) -> Hsla {
    let rgb = match color {
        Color::Spec(value) => value,
        Color::Named(name) => screen.colors[name as usize],
        Color::Indexed(index) => screen.colors[usize::from(index)],
    };
    hsla_from_rgb(rgb)
}

pub(super) fn hsla_from_rgb(Rgb { r, g, b }: Rgb) -> Hsla {
    rgb(u32::from_be_bytes([0, r, g, b])).into()
}

pub(super) fn selection(theme: TerminalTheme) -> Hsla {
    match theme {
        TerminalTheme::Light => rgb(0x7850dc).alpha(0.15).into(),
        TerminalTheme::Dark => hsla(260. / 360., 0.7, 0.6, 0.3),
    }
}

pub(super) struct CellStyle {
    pub font: Font,
    pub foreground: Hsla,
    pub background: Hsla,
    pub underline: Hsla,
}

impl CellStyle {
    pub fn new(
        screen: &TerminalSnapshot,
        cell: &TerminalCell,
        base: &Font,
        selection: Hsla,
    ) -> Self {
        let mut font = base.clone();
        let mut ink = cell.foreground;
        if cell.flags.contains(CellFlags::BOLD) {
            font.weight = FontWeight::BOLD;
            ink = match ink {
                Color::Named(name) if (name as usize) < 8 => Color::Indexed(name as u8 + 8),
                Color::Indexed(index @ 0..=7) => Color::Indexed(index + 8),
                _ => ink,
            };
        }
        if cell.flags.contains(CellFlags::ITALIC) {
            font.style = FontStyle::Italic;
        }
        let mut foreground = color(screen, ink);
        let mut background = color(screen, cell.background);
        if cell.flags.contains(CellFlags::INVERSE) {
            std::mem::swap(&mut foreground, &mut background);
        }
        if cell.flags.contains(CellFlags::DIM) {
            foreground = background.blend(foreground.opacity(0.5));
        }
        if cell.selected {
            background = background.blend(selection);
        }
        let underline = cell
            .underline_color
            .map_or(foreground, |c| color(screen, c));
        Self {
            font,
            foreground,
            background,
            underline,
        }
    }
}

pub(super) fn background(screen: &TerminalSnapshot) -> Hsla {
    color(screen, Color::Named(NamedColor::Background))
}
