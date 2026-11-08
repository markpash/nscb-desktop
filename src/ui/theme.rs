//! The NSCB dark palette, applied on top of the gpui-kit dark theme.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{rgb, rgba, App, Hsla};

pub fn hex(v: u32) -> Hsla {
    rgb(v).into()
}

/// `v` as 0xRRGGBB with alpha in 0..=1.
pub fn hexa(v: u32, alpha: f32) -> Hsla {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
    rgba((v << 8) | a).into()
}

pub const BG_PRIMARY: u32 = 0x0b0e14;
pub const BG_SECONDARY: u32 = 0x111620;
pub const BG_TERTIARY: u32 = 0x171c28;
pub const BG_ELEVATED: u32 = 0x1a2030;

pub const ACCENT: u32 = 0x00d4aa;
pub const ACCENT_2: u32 = 0x00b4d8;
pub const DANGER: u32 = 0xf85149;
pub const WARNING: u32 = 0xd29922;
pub const SUCCESS: u32 = 0x3fb950;

pub const TEXT_PRIMARY: u32 = 0xe6edf3;
pub const TEXT_SECONDARY: u32 = 0x8b949e;
pub const TEXT_MUTED: u32 = 0x484f58;

pub fn border() -> Hsla {
    hexa(0xffffff, 0.06)
}

pub fn border_hover() -> Hsla {
    hexa(0xffffff, 0.12)
}

/// Accent color for each operation page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent {
    Compress,
    Decompress,
    Merge,
    Convert,
    Split,
    Dspl,
    Create,
    Info,
    Rename,
    Settings,
    BatchMerge,
    Verify,
}

impl Accent {
    pub fn color(self) -> u32 {
        match self {
            Accent::Compress => 0x00d4aa,
            Accent::Decompress => 0x00b4d8,
            Accent::Merge => 0xd2a8ff,
            Accent::Convert => 0xf0883e,
            Accent::Split => 0xf778ba,
            Accent::Dspl => 0xe0758a,
            Accent::Create => 0x58a6ff,
            Accent::Info => 0x79c0ff,
            Accent::Rename => 0xffa657,
            Accent::Settings => 0x8b949e,
            Accent::BatchMerge => 0x56d8a4,
            Accent::Verify => 0x63e6be,
        }
    }
}

/// Badge color for a file extension (or `FOLDER`).
pub fn ext_color(ext: &str) -> u32 {
    match ext {
        "NSP" => 0x3fb950,
        "XCI" => 0x58a6ff,
        "NSZ" => 0xd2a8ff,
        "XCZ" => 0xf0883e,
        "NCZ" => 0xf778ba,
        "NCA" => 0x8b949e,
        "FOLDER" => 0xe3b341,
        _ => TEXT_MUTED,
    }
}

pub fn apply(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    // Colors are edited in a second update after the mode switch (see Theme::update).
    Theme::update(cx, |theme| {
        let c = &mut theme.colors;
        c.background = hex(BG_PRIMARY);
        c.foreground = hex(TEXT_PRIMARY);
        c.border = border();
        c.input = border_hover();
        c.ring = hexa(ACCENT, 0.5);
        c.caret = hex(ACCENT);
        c.selection = hexa(ACCENT, 0.25);
        c.muted = hex(BG_TERTIARY);
        c.muted_foreground = hex(TEXT_SECONDARY);

        c.primary = hex(ACCENT);
        c.primary_hover = hex(0x1ee0bb);
        c.primary_active = hex(0x00b892);
        c.primary_foreground = hex(BG_PRIMARY);
        c.button_primary = c.primary;
        c.button_primary_hover = c.primary_hover;
        c.button_primary_active = c.primary_active;
        c.button_primary_foreground = c.primary_foreground;

        c.secondary = hex(BG_ELEVATED);
        c.secondary_hover = hex(0x222a3d);
        c.secondary_active = hex(0x283147);
        c.secondary_foreground = hex(TEXT_PRIMARY);
        c.button_secondary = c.secondary;
        c.button_secondary_hover = c.secondary_hover;
        c.button_secondary_active = c.secondary_active;
        c.button_secondary_foreground = c.secondary_foreground;

        c.danger = hex(DANGER);
        c.danger_hover = hex(0xff6a62);
        c.danger_active = hex(0xe03e36);
        c.danger_foreground = hex(0xffffff);
        c.success = hex(SUCCESS);
        c.warning = hex(WARNING);

        c.popover = hex(BG_ELEVATED);
        c.popover_foreground = hex(TEXT_PRIMARY);
        c.accent = hexa(ACCENT, 0.12);
        c.accent_foreground = hex(TEXT_PRIMARY);
        c.list_active = hexa(ACCENT, 0.15);
        c.list_active_border = hexa(ACCENT, 0.3);
        c.list_hover = hex(BG_TERTIARY);

        c.switch = hex(BG_ELEVATED);
        c.switch_thumb = hex(TEXT_PRIMARY);
        c.slider_bar = hex(ACCENT);
        c.slider_thumb = hex(TEXT_PRIMARY);
        c.progress_bar = hex(ACCENT);

        c.tab_bar = hex(BG_TERTIARY);
        c.tab_bar_segmented = hex(BG_TERTIARY);
        c.tab_active = hexa(ACCENT, 0.15);
        c.tab_active_foreground = hex(ACCENT);
        c.tab_foreground = hex(TEXT_SECONDARY);

        c.scrollbar = gpui_kit::transparent_black();
        c.scrollbar_thumb = hexa(0xffffff, 0.12);
        c.scrollbar_thumb_hover = hexa(0xffffff, 0.2);
        c.overlay = hexa(0x000000, 0.5);
        theme.radius = gpui_kit::px(6.);
        theme.radius_lg = gpui_kit::px(10.);
        theme.font_size = gpui_kit::px(14.);
    });
}
