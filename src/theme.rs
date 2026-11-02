//! Colors of the chrome around the canvas. Dark, to match pizarra's default
//! black board, with the green of pizarra's icons as accent.

use gpui::{Hsla, Rgba, rgb, rgba};
use pizarra::color::Color;

pub fn toolbar_bg() -> Rgba {
    rgb(0x232326)
}
pub fn popover_bg() -> Rgba {
    rgb(0x2c2c30)
}
pub fn border() -> Rgba {
    rgb(0x3b3b41)
}
pub fn hover() -> Rgba {
    rgb(0x37373d)
}
pub fn pressed() -> Rgba {
    rgb(0x45454c)
}
pub fn text() -> Rgba {
    rgb(0xe9e9ec)
}
pub fn muted() -> Rgba {
    rgb(0x9d9da6)
}
pub fn accent() -> Rgba {
    rgb(0xa9db77)
}
pub fn accent_text() -> Rgba {
    rgb(0x1b2412)
}
pub fn danger() -> Rgba {
    rgb(0xf66151)
}
pub fn scrim() -> Rgba {
    rgba(0x0000_0099)
}

pub fn to_hsla(c: Color) -> Hsla {
    Rgba { r: c.r as f32 / 255.0, g: c.g as f32 / 255.0, b: c.b as f32 / 255.0, a: c.a as f32 / 255.0 }.into()
}

/// The GNOME palette, the one GTK's color chooser shows by default: one column
/// per hue, from light to dark.
pub const PALETTE: [[u32; 5]; 9] = [
    [0x99c1f1, 0x62a0ea, 0x3584e4, 0x1c71d8, 0x1a5fb4],
    [0x8ff0a4, 0x57e389, 0x33d17a, 0x2ec27e, 0x26a269],
    [0xf9f06b, 0xf8e45c, 0xf6d32d, 0xf5c211, 0xe5a50a],
    [0xffbe6f, 0xffa348, 0xff7800, 0xe66100, 0xc64600],
    [0xf66151, 0xed333b, 0xe01b24, 0xc01c28, 0xa51d2d],
    [0xdc8add, 0xc061cb, 0x9141ac, 0x813d9c, 0x613583],
    [0xcdab8f, 0xb5835a, 0x986a44, 0x865e3c, 0x63452c],
    [0xffffff, 0xf6f5f4, 0xdeddda, 0xc0bfbc, 0x9a9996],
    [0x77767b, 0x5e5c64, 0x3d3846, 0x241f31, 0x000000],
];

pub fn palette_color(hex: u32) -> Color {
    Color::from_int_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}
