use super::{NamedColor, Rgb, TerminalTheme};
use alacritty_terminal::term::color::COUNT;

const LIGHT: [u32; 16] = [
    0x1a1830, 0xdc2626, 0x16a34a, 0xca8a04, 0x4f46e5, 0x9333ea, 0x0891b2, 0xf5f3ff, 0x8b85a0,
    0xef4444, 0x22c55e, 0xeab308, 0x6366f1, 0xa855f7, 0x06b6d4, 0xffffff,
];
const DARK: [u32; 16] = [
    0x16141f, 0xef4444, 0x34d399, 0xfbbf24, 0x818cf8, 0xc084fc, 0x22d3ee, 0xe2e0eb, 0x6b6880,
    0xf87171, 0x6ee7b7, 0xfde68a, 0xa5b4fc, 0xd8b4fe, 0x67e8f9, 0xf5f3ff,
];

pub(super) fn colors(theme: TerminalTheme) -> [Rgb; COUNT] {
    let mut colors = [Rgb::default(); COUNT];
    let ansi = match theme {
        TerminalTheme::Light => LIGHT,
        TerminalTheme::Dark => DARK,
    };
    for (index, value) in ansi.into_iter().enumerate() {
        colors[index] = rgb(value);
    }
    let levels = [0, 95, 135, 175, 215, 255];
    for index in 0..216 {
        colors[16 + index] = Rgb {
            r: levels[index / 36],
            g: levels[(index / 6) % 6],
            b: levels[index % 6],
        };
    }
    for index in 0..24 {
        let value = 8 + index as u8 * 10;
        colors[232 + index] = Rgb {
            r: value,
            g: value,
            b: value,
        };
    }
    let (foreground, cursor) = match theme {
        TerminalTheme::Light => (0x1a1830, 0x6226d9),
        TerminalTheme::Dark => (0xdedde3, 0x9167e4),
    };
    colors[NamedColor::Foreground as usize] = rgb(foreground);
    colors[NamedColor::Background as usize] = theme.default_background();
    colors[NamedColor::Cursor as usize] = rgb(cursor);
    for index in 0..8 {
        colors[NamedColor::DimBlack as usize + index] = colors[index] * (2. / 3.);
    }
    colors[NamedColor::BrightForeground as usize] = colors[NamedColor::Foreground as usize];
    colors[NamedColor::DimForeground as usize] =
        colors[NamedColor::Foreground as usize] * (2. / 3.);
    colors
}

impl TerminalTheme {
    pub(crate) fn default_background(self) -> Rgb {
        rgb(match self {
            Self::Light => 0xfaf9ff,
            Self::Dark => 0x17161d,
        })
    }
}

fn rgb(value: u32) -> Rgb {
    Rgb {
        r: (value >> 16) as u8,
        g: (value >> 8) as u8,
        b: value as u8,
    }
}
