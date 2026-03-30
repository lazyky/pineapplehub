//! Centralised PineappleHub theme — multi-palette system with runtime switching.
//!
//! All colour constants and reusable style functions live here so that
//! individual view files never hard-code colours inline.
//!
//! ## How it works
//! - [`ColorPalette`] holds every colour token for one theme.
//! - Four `static` palettes are defined: [`PALETTE_CLASSIC`], [`PALETTE_PINEAPPLE`],
//!   [`PALETTE_OCEAN`], [`PALETTE_FOREST`].
//! - A `thread_local!` cell stores the active palette pointer.
//! - [`set_active_palette`] switches the active palette; all style functions
//!   and colour accessors read from it automatically — zero signature changes.

use iced::{
    color,
    widget::{container, text_input},
    Border, Color, Theme,
};

// ──────────────────────────────  Palette struct  ──────────────────────────────

/// Chart-specific colours (plotters crate uses its own `RGBColor`).
pub struct ChartPalette {
    pub bg: plotters::prelude::RGBColor,
    pub axis: plotters::prelude::RGBColor,
    pub label: plotters::prelude::RGBColor,
    pub tick: plotters::prelude::RGBColor,
    pub normal_line: plotters::prelude::RGBAColor,
    pub suspect_line: plotters::prelude::RGBAColor,
    pub highlight_line: plotters::prelude::RGBAColor,
    pub tooltip_bg: plotters::prelude::RGBColor,
    pub tooltip_border: plotters::prelude::RGBColor,
    pub tooltip_header: plotters::prelude::RGBColor,
    pub legend_normal: plotters::prelude::RGBColor,
    pub legend_suspect: plotters::prelude::RGBColor,
}

/// A complete colour palette for one theme variant.
pub struct ColorPalette {
    // Background layers
    pub bg_deep: Color,
    pub bg_base: Color,
    pub bg_surface: Color,
    pub bg_elevated: Color,

    // Text
    pub text_primary: Color,

    // Accent + derived
    pub accent: Color,
    pub accent_dim: Color,

    // Semantic
    pub success: Color,
    pub warning: Color,
    pub danger: Color,

    // Table
    pub row_alt: Color,
    pub row_suspect: Color,
    pub cell_outlier: Color,
    pub row_flash: Color,

    // Borders
    pub border_subtle: Color,
    pub border_accent: Color,

    // Text colours
    pub outlier_text: Color,

    // Chart sub-palette
    pub chart: ChartPalette,
}

// ──────────────────────────────  Theme variant  ──────────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ThemeVariant {
    /// Original PornHub-inspired orange (default).
    #[default]
    Classic,
    /// Pineapple gold skin.
    Pineapple,
    /// Deep sea blue.
    Ocean,
    /// Emerald green.
    Forest,
}

impl ThemeVariant {
    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Pineapple => "Pineapple",
            Self::Ocean => "Ocean",
            Self::Forest => "Forest",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Classic => Self::Pineapple,
            Self::Pineapple => Self::Ocean,
            Self::Ocean => Self::Forest,
            Self::Forest => Self::Classic,
        }
    }

    pub fn palette(self) -> &'static ColorPalette {
        match self {
            Self::Classic => &PALETTE_CLASSIC,
            Self::Pineapple => &PALETTE_PINEAPPLE,
            Self::Ocean => &PALETTE_OCEAN,
            Self::Forest => &PALETTE_FOREST,
        }
    }
}

// ──────────────────────────────  Static palettes  ─────────────────────────────

// Helper: build a `Color` with a specific alpha from an RGB tuple
const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
}

// ── Classic (PornHub orange) ──────────────────────────────────────────────────

static PALETTE_CLASSIC: ColorPalette = ColorPalette {
    bg_deep: color!(0x000000),
    bg_base: color!(0x0d0d0d),
    bg_surface: color!(0x1a1a1a),
    bg_elevated: color!(0x272727),
    text_primary: color!(0xffffff),
    accent: color!(0xff9900),
    accent_dim: rgba(0xff, 0x99, 0x00, 0.25),
    success: color!(0x4caf50),
    warning: color!(0xff9900),
    danger: color!(0xf44336),
    row_alt: color!(0x1a1a1a),
    row_suspect: rgba(0xff, 0x99, 0x00, 0.12),
    cell_outlier: rgba(0xf4, 0x43, 0x36, 0.22),
    row_flash: rgba(0xff, 0x99, 0x00, 0.35),
    border_subtle: rgba(0x44, 0x44, 0x44, 0.60),
    border_accent: rgba(0xff, 0x99, 0x00, 0.80),
    outlier_text: rgba(0xf2, 0x4c, 0x4c, 1.0),
    chart: ChartPalette {
        bg: plotters::prelude::RGBColor(0x0d, 0x0d, 0x0d),
        axis: plotters::prelude::RGBColor(0x44, 0x44, 0x44),
        label: plotters::prelude::RGBColor(0xcc, 0xcc, 0xcc),
        tick: plotters::prelude::RGBColor(0x88, 0x88, 0x88),
        normal_line: plotters::prelude::RGBAColor(150, 180, 220, 0.22),
        suspect_line: plotters::prelude::RGBAColor(244, 67, 54, 0.75),
        highlight_line: plotters::prelude::RGBAColor(255, 153, 0, 1.0),
        tooltip_bg: plotters::prelude::RGBColor(0x27, 0x27, 0x27),
        tooltip_border: plotters::prelude::RGBColor(0x44, 0x44, 0x44),
        tooltip_header: plotters::prelude::RGBColor(0xff, 0x99, 0x00),
        legend_normal: plotters::prelude::RGBColor(150, 180, 220),
        legend_suspect: plotters::prelude::RGBColor(244, 67, 54),
    },
};

// ── Pineapple (gold skin) ─────────────────────────────────────────────────────

static PALETTE_PINEAPPLE: ColorPalette = ColorPalette {
    bg_deep: color!(0x0c0e08),
    bg_base: color!(0x141810),
    bg_surface: color!(0x1e2418),
    bg_elevated: color!(0x2a3220),
    text_primary: color!(0xffffff),
    accent: color!(0xe8b830),
    accent_dim: rgba(0xe8, 0xb8, 0x30, 0.25),
    success: color!(0x5a9e3e),
    warning: color!(0xf0c040),
    danger: color!(0xe05545),
    row_alt: color!(0x1e2418),
    row_suspect: rgba(0xe8, 0xb8, 0x30, 0.12),
    cell_outlier: rgba(0xe0, 0x55, 0x45, 0.22),
    row_flash: rgba(0xe8, 0xb8, 0x30, 0.35),
    border_subtle: rgba(0x44, 0x55, 0x33, 0.60),
    border_accent: rgba(0xe8, 0xb8, 0x30, 0.80),
    outlier_text: rgba(0xe0, 0x55, 0x45, 1.0),
    chart: ChartPalette {
        bg: plotters::prelude::RGBColor(0x14, 0x18, 0x10),
        axis: plotters::prelude::RGBColor(0x44, 0x55, 0x33),
        label: plotters::prelude::RGBColor(0xcc, 0xcc, 0xaa),
        tick: plotters::prelude::RGBColor(0x88, 0x99, 0x77),
        normal_line: plotters::prelude::RGBAColor(150, 200, 130, 0.22),
        suspect_line: plotters::prelude::RGBAColor(224, 85, 69, 0.75),
        highlight_line: plotters::prelude::RGBAColor(232, 184, 48, 1.0),
        tooltip_bg: plotters::prelude::RGBColor(0x2a, 0x32, 0x20),
        tooltip_border: plotters::prelude::RGBColor(0x44, 0x55, 0x33),
        tooltip_header: plotters::prelude::RGBColor(0xe8, 0xb8, 0x30),
        legend_normal: plotters::prelude::RGBColor(150, 200, 130),
        legend_suspect: plotters::prelude::RGBColor(224, 85, 69),
    },
};

// ── Ocean (deep sea blue) ─────────────────────────────────────────────────────

static PALETTE_OCEAN: ColorPalette = ColorPalette {
    bg_deep: color!(0x0a0e1a),
    bg_base: color!(0x0f1629),
    bg_surface: color!(0x182240),
    bg_elevated: color!(0x1f2d52),
    text_primary: color!(0xffffff),
    accent: color!(0x3b9dff),
    accent_dim: rgba(0x3b, 0x9d, 0xff, 0.25),
    success: color!(0x4caf50),
    warning: color!(0xffb74d),
    danger: color!(0xef5350),
    row_alt: color!(0x182240),
    row_suspect: rgba(0x3b, 0x9d, 0xff, 0.12),
    cell_outlier: rgba(0xef, 0x53, 0x50, 0.22),
    row_flash: rgba(0x3b, 0x9d, 0xff, 0.35),
    border_subtle: rgba(0x33, 0x44, 0x66, 0.60),
    border_accent: rgba(0x3b, 0x9d, 0xff, 0.80),
    outlier_text: rgba(0xef, 0x53, 0x50, 1.0),
    chart: ChartPalette {
        bg: plotters::prelude::RGBColor(0x0f, 0x16, 0x29),
        axis: plotters::prelude::RGBColor(0x33, 0x44, 0x66),
        label: plotters::prelude::RGBColor(0xaa, 0xbb, 0xdd),
        tick: plotters::prelude::RGBColor(0x66, 0x77, 0x99),
        normal_line: plotters::prelude::RGBAColor(100, 160, 230, 0.22),
        suspect_line: plotters::prelude::RGBAColor(239, 83, 80, 0.75),
        highlight_line: plotters::prelude::RGBAColor(59, 157, 255, 1.0),
        tooltip_bg: plotters::prelude::RGBColor(0x1f, 0x2d, 0x52),
        tooltip_border: plotters::prelude::RGBColor(0x33, 0x44, 0x66),
        tooltip_header: plotters::prelude::RGBColor(0x3b, 0x9d, 0xff),
        legend_normal: plotters::prelude::RGBColor(100, 160, 230),
        legend_suspect: plotters::prelude::RGBColor(239, 83, 80),
    },
};

// ── Forest (emerald green) ────────────────────────────────────────────────────

static PALETTE_FOREST: ColorPalette = ColorPalette {
    bg_deep: color!(0x0a1210),
    bg_base: color!(0x0f1a16),
    bg_surface: color!(0x162a23),
    bg_elevated: color!(0x1e3a30),
    text_primary: color!(0xffffff),
    accent: color!(0x4ade80),
    accent_dim: rgba(0x4a, 0xde, 0x80, 0.25),
    success: color!(0x22c55e),
    warning: color!(0xfbbf24),
    danger: color!(0xf87171),
    row_alt: color!(0x162a23),
    row_suspect: rgba(0x4a, 0xde, 0x80, 0.12),
    cell_outlier: rgba(0xf8, 0x71, 0x71, 0.22),
    row_flash: rgba(0x4a, 0xde, 0x80, 0.35),
    border_subtle: rgba(0x33, 0x55, 0x44, 0.60),
    border_accent: rgba(0x4a, 0xde, 0x80, 0.80),
    outlier_text: rgba(0xf8, 0x71, 0x71, 1.0),
    chart: ChartPalette {
        bg: plotters::prelude::RGBColor(0x0f, 0x1a, 0x16),
        axis: plotters::prelude::RGBColor(0x33, 0x55, 0x44),
        label: plotters::prelude::RGBColor(0xaa, 0xcc, 0xbb),
        tick: plotters::prelude::RGBColor(0x66, 0x99, 0x77),
        normal_line: plotters::prelude::RGBAColor(100, 210, 140, 0.22),
        suspect_line: plotters::prelude::RGBAColor(248, 113, 113, 0.75),
        highlight_line: plotters::prelude::RGBAColor(74, 222, 128, 1.0),
        tooltip_bg: plotters::prelude::RGBColor(0x1e, 0x3a, 0x30),
        tooltip_border: plotters::prelude::RGBColor(0x33, 0x55, 0x44),
        tooltip_header: plotters::prelude::RGBColor(0x4a, 0xde, 0x80),
        legend_normal: plotters::prelude::RGBColor(100, 210, 140),
        legend_suspect: plotters::prelude::RGBColor(248, 113, 113),
    },
};

// ──────────────────────────────  Thread-local active palette  ─────────────────

use std::cell::Cell;

thread_local! {
    static ACTIVE_VARIANT: Cell<ThemeVariant> = const { Cell::new(ThemeVariant::Classic) };
}

/// Switch the active theme palette. Call this from `App::update` when handling
/// `Message::SwitchTheme`.
pub fn set_active_palette(variant: ThemeVariant) {
    ACTIVE_VARIANT.with(|c| c.set(variant));
}

/// Return the currently active colour palette.
#[inline]
pub fn active_palette() -> &'static ColorPalette {
    ACTIVE_VARIANT.with(|c| c.get()).palette()
}

/// Return the currently active theme variant.
#[allow(dead_code)]
#[inline]
pub fn active_variant() -> ThemeVariant {
    ACTIVE_VARIANT.with(|c| c.get())
}

// ──────────────────────────────  Colour accessors  ────────────────────────────
// These mirror the old `pub const` names so call sites only need `()` appended.
// Some are not yet used at all sites — that's expected for a public API.
#[allow(dead_code)]
pub fn bg_deep() -> Color { active_palette().bg_deep }
#[allow(dead_code)]
pub fn bg_base() -> Color { active_palette().bg_base }
#[allow(dead_code)]
pub fn bg_surface() -> Color { active_palette().bg_surface }
#[allow(dead_code)]
pub fn bg_elevated() -> Color { active_palette().bg_elevated }
pub fn text_primary() -> Color { active_palette().text_primary }
pub fn accent() -> Color { active_palette().accent }
#[allow(dead_code)]
pub fn accent_dim() -> Color { active_palette().accent_dim }
pub fn success() -> Color { active_palette().success }
pub fn warning() -> Color { active_palette().warning }
pub fn danger() -> Color { active_palette().danger }
pub fn outlier_text() -> Color { active_palette().outlier_text }


// ────────────────────────  Iced Theme  ────────────────────────

/// Build the app-wide Iced `Theme` from the currently active palette.
pub fn pineapple_theme() -> Theme {
    use iced::theme::palette;

    let p = active_palette();
    let custom_palette = palette::Palette {
        background: p.bg_base,
        text: p.text_primary,
        primary: p.accent,
        success: p.success,
        warning: p.warning,
        danger: p.danger,
    };

    Theme::custom_with_fn("PineappleHub", custom_palette, |palette| {
        let mut ext = palette::Extended::generate(palette);
        ext.background.base.color = p.bg_base;
        ext.background.weak.color = p.bg_surface;
        ext.background.strong.color = p.bg_elevated;
        ext
    })
}

// ────────────────────────  Style Functions  ────────────────────────

/// Title bar: deep background + bottom accent line.
pub fn title_bar_style(_theme: &Theme) -> container::Style {
    let p = active_palette();
    container::Style {
        background: Some(iced::Background::Color(p.bg_deep)),
        border: Border {
            color: p.border_accent,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

/// "Hub" badge: accent background with rounded corners.
pub fn hub_badge_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(active_palette().accent)),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

/// Sidebar pane: slightly elevated surface + right border.
pub fn sidebar_style(_theme: &Theme) -> container::Style {
    let p = active_palette();
    container::Style {
        background: Some(iced::Background::Color(p.bg_deep)),
        border: Border {
            color: p.border_subtle,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

/// Tab bar background.
pub fn tab_bar_style(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(active_palette().bg_surface)),
        border: Border {
            width: 0.0,
            ..Default::default()
        },
        ..container::transparent(theme)
    }
}

/// Active tab accent underline (3px).
pub fn active_tab_underline(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(active_palette().accent)),
        ..Default::default()
    }
}

/// Table header row: elevated background + bottom hairline.
pub fn table_header_style(_theme: &Theme) -> container::Style {
    let p = active_palette();
    container::Style {
        background: Some(iced::Background::Color(p.bg_elevated)),
        border: Border {
            color: p.border_subtle,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

/// Gold accent separator line (1px high container).
pub fn accent_separator(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(active_palette().border_accent)),
        ..Default::default()
    }
}

/// Section card: wraps charts, stat tables, etc. with subtle border + roundness.
pub fn section_card_style(_theme: &Theme) -> container::Style {
    let p = active_palette();
    container::Style {
        background: Some(iced::Background::Color(p.bg_surface)),
        border: Border {
            color: p.border_subtle,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

/// Table data row with alternating colour.
pub fn table_row_bg(
    index: usize,
    is_suspect: bool,
    is_flash_on: bool,
) -> impl Fn(&Theme) -> container::Style {
    move |theme: &Theme| {
        let p = active_palette();
        let bg = if is_flash_on {
            p.row_flash
        } else if is_suspect {
            p.row_suspect
        } else if index % 2 == 1 {
            p.row_alt
        } else {
            Color::TRANSPARENT
        };
        container::Style {
            background: Some(iced::Background::Color(bg)),
            ..container::transparent(theme)
        }
    }
}

/// Outlier metric cell highlight.
pub fn outlier_cell_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(active_palette().cell_outlier)),
        ..Default::default()
    }
}

/// Summary card for statistics panel.
pub fn summary_card_style(accent: Color) -> impl Fn(&Theme) -> container::Style {
    move |theme: &Theme| container::Style {
        background: Some(iced::Background::Color(Color {
            a: 0.10,
            ..accent
        })),
        border: Border {
            color: Color { a: 0.30, ..accent },
            width: 1.0,
            radius: 10.0.into(),
        },
        ..container::transparent(theme)
    }
}

/// Statistics table header row.
pub fn stats_header_style(_theme: &Theme) -> container::Style {
    let p = active_palette();
    container::Style {
        background: Some(iced::Background::Color(p.bg_elevated)),
        border: Border {
            color: p.border_subtle,
            width: 0.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

/// Tooltip: dark, elevated, rounded.
pub fn tooltip_style(_theme: &Theme) -> container::Style {
    let p = active_palette();
    container::Style {
        background: Some(iced::Background::Color(p.bg_elevated)),
        text_color: Some(p.text_primary),
        border: Border {
            color: p.border_subtle,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    }
}

/// Full-screen overlay (decode-in-progress).
pub fn overlay_style(_theme: &Theme) -> container::Style {
    let p = active_palette();
    container::Style {
        background: Some(iced::Background::Color(Color {
            a: 0.70,
            ..p.bg_deep
        })),
        text_color: Some(p.text_primary),
        ..Default::default()
    }
}

/// Inline editor container (note / metric edit).
pub fn editor_container_style(_theme: &Theme) -> container::Style {
    let p = active_palette();
    container::Style {
        background: Some(iced::Background::Color(p.bg_surface)),
        border: Border {
            color: p.border_subtle,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

/// Dialog box (export-delete prompt, etc.).
pub fn dialog_box_style(_theme: &Theme) -> container::Style {
    let p = active_palette();
    container::Style {
        background: Some(iced::Background::Color(p.bg_elevated)),
        border: Border {
            color: p.border_subtle,
            width: 1.0,
            radius: 12.0.into(),
        },
        ..Default::default()
    }
}

/// Dialog scrim (semi-transparent background behind dialogs).
pub fn dialog_scrim_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(Color {
            a: 0.60,
            ..active_palette().bg_deep
        })),
        ..Default::default()
    }
}

/// Selected file item in analysis page job list.
pub fn selected_job_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(active_palette().accent_dim)),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    }
}

/// Validation error text_input style.
pub fn validation_error_input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let mut s = text_input::default(theme, status);
    s.border.color = active_palette().danger;
    s.border.width = 2.0;
    s
}

/// Undo toast container.
pub fn undo_toast_style(_theme: &Theme) -> container::Style {
    let p = active_palette();
    container::Style {
        background: Some(iced::Background::Color(p.bg_elevated)),
        border: Border {
            color: p.border_subtle,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

/// Cache warning banner.
pub fn cache_warning_style(_theme: &Theme) -> container::Style {
    let p = active_palette();
    container::Style {
        background: Some(iced::Background::Color(p.bg_surface)),
        border: Border {
            color: Color { a: 0.40, ..p.warning },
            width: 1.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    }
}

// ────────────────────────  Button styles  ────────────────────────

use iced::widget::button;

/// Global button border-radius used for all custom button variants.
const BTN_RADIUS: f32 = 8.0;

/// Helper: take an iced-generated button style and force our border-radius.
fn rounded(mut s: button::Style) -> button::Style {
    s.border.radius = BTN_RADIUS.into();
    s
}

/// Standard primary button with rounded corners.
pub fn primary_button_style(theme: &Theme, status: button::Status) -> button::Style {
    rounded(button::primary(theme, status))
}

/// Standard secondary button with rounded corners.
pub fn secondary_button_style(theme: &Theme, status: button::Status) -> button::Style {
    rounded(button::secondary(theme, status))
}

/// Transparent / text button with rounded corners (visible on hover).
pub fn text_button_style(theme: &Theme, status: button::Status) -> button::Style {
    rounded(button::text(theme, status))
}

/// Sidebar cleanup button: dark surface with generous border-radius.
pub fn cleanup_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let p = active_palette();
    let base = button::Style {
        background: Some(iced::Background::Color(p.bg_elevated)),
        text_color: p.text_primary,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: BTN_RADIUS.into(),
        },
        ..Default::default()
    };
    match status {
        button::Status::Hovered => button::Style {
            background: Some(iced::Background::Color(p.bg_surface)),
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(iced::Background::Color(p.border_subtle)),
            ..base
        },
        _ => base,
    }
}

/// Accent / primary action button: accent background, dark text, generous radius.
pub fn danger_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let p = active_palette();
    let ac = p.accent;
    let base = button::Style {
        background: Some(iced::Background::Color(ac)),
        text_color: Color::BLACK,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: BTN_RADIUS.into(),
        },
        ..Default::default()
    };
    match status {
        button::Status::Hovered => button::Style {
            background: Some(iced::Background::Color(Color {
                a: 0.85,
                ..ac
            })),
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(iced::Background::Color(Color {
                a: 0.65,
                ..ac
            })),
            ..base
        },
        _ => base,
    }
}

// ────────────────────────  Chart palette accessors  ────────────────────────
// The `plotters` crate uses its own colour types, so we expose typed accessors
// instead of the old `pub mod chart { pub const ... }` constants.

pub mod chart {
    use super::active_palette;
    use plotters::prelude::{RGBAColor, RGBColor};

    pub fn bg() -> RGBColor { active_palette().chart.bg }
    pub fn axis() -> RGBColor { active_palette().chart.axis }
    pub fn label() -> RGBColor { active_palette().chart.label }
    pub fn tick() -> RGBColor { active_palette().chart.tick }
    pub fn normal_line() -> RGBAColor { active_palette().chart.normal_line }
    pub fn suspect_line() -> RGBAColor { active_palette().chart.suspect_line }
    pub fn highlight_line() -> RGBAColor { active_palette().chart.highlight_line }
    pub fn tooltip_bg() -> RGBColor { active_palette().chart.tooltip_bg }
    pub fn tooltip_border() -> RGBColor { active_palette().chart.tooltip_border }
    pub fn tooltip_header() -> RGBColor { active_palette().chart.tooltip_header }
    pub fn legend_normal() -> RGBColor { active_palette().chart.legend_normal }
    pub fn legend_suspect() -> RGBColor { active_palette().chart.legend_suspect }
}
