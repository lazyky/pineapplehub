//! Centralised PineappleHub theme — premium dark palette inspired by the mockup.
//!
//! All colour constants and reusable style functions live here so that
//! individual view files never hard-code colours inline.

use iced::{
    color,
    widget::{container, text_input},
    Border, Color, Theme,
};

// ────────────────────────  Colour Constants  ────────────────────────

// Background layers (deepest → most elevated)  –  pure black family
pub const BG_DEEP: Color = color!(0x000000);
pub const BG_BASE: Color = color!(0x0d0d0d);
pub const BG_SURFACE: Color = color!(0x1a1a1a);
pub const BG_ELEVATED: Color = color!(0x272727);

// Text hierarchy  –  white / light grey
pub const TEXT_PRIMARY: Color = color!(0xffffff);


// Accent  –  PornHub orange
pub const ACCENT: Color = color!(0xff9900);

pub const ACCENT_DIM: Color = Color {
    r: 0xff as f32 / 255.0,
    g: 0x99 as f32 / 255.0,
    b: 0x00 as f32 / 255.0,
    a: 0.25,
};

// Semantic
pub const SUCCESS: Color = color!(0x4caf50);
pub const WARNING: Color = color!(0xff9900);
pub const DANGER: Color = color!(0xf44336);

// Table
pub const ROW_ALT: Color = color!(0x1a1a1a);
/// Suspect row — warm amber tint (overrides alternating row colour).
pub const ROW_SUSPECT: Color = Color {
    r: 0xff as f32 / 255.0,
    g: 0x99 as f32 / 255.0,
    b: 0x00 as f32 / 255.0,
    a: 0.12,
};
/// Outlier cell — soft red highlight inside a row.
pub const CELL_OUTLIER: Color = Color {
    r: 0xf4 as f32 / 255.0,
    g: 0x43 as f32 / 255.0,
    b: 0x36 as f32 / 255.0,
    a: 0.22,
};
/// Flash-highlight colour (accent translucent).
pub const ROW_FLASH: Color = Color {
    r: 0xff as f32 / 255.0,
    g: 0x99 as f32 / 255.0,
    b: 0x00 as f32 / 255.0,
    a: 0.35,
};

// Borders / separators
pub const BORDER_SUBTLE: Color = Color {
    r: 0x44 as f32 / 255.0,
    g: 0x44 as f32 / 255.0,
    b: 0x44 as f32 / 255.0,
    a: 0.60,
};
pub const BORDER_ACCENT: Color = Color {
    r: 0xff as f32 / 255.0,
    g: 0x99 as f32 / 255.0,
    b: 0x00 as f32 / 255.0,
    a: 0.80,
};

// ────────────────────────  Iced Theme  ────────────────────────

/// Build the app-wide Iced `Theme` with our custom palette.
pub fn pineapple_theme() -> Theme {
    use iced::theme::palette;

    let custom_palette = palette::Palette {
        background: BG_BASE,
        text: TEXT_PRIMARY,
        primary: ACCENT,
        success: SUCCESS,
        warning: WARNING,
        danger: DANGER,
    };

    Theme::custom_with_fn("PineappleHub", custom_palette, |palette| {
        // Start from the built-in generator, then tweak the extended palette.
        let mut ext = palette::Extended::generate(palette);

        // Make the "secondary" surface colours a bit lighter so buttons
        // and inputs stand out against the dark background.
        ext.background.base.color = BG_BASE;
        ext.background.weak.color = BG_SURFACE;
        ext.background.strong.color = BG_ELEVATED;

        ext
    })
}

// ────────────────────────  Style Functions  ────────────────────────

/// Title bar: deep background + bottom accent line.
pub fn title_bar_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG_DEEP)),
        border: Border {
            color: BORDER_ACCENT,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

/// "Hub" badge: orange background with rounded corners (PornHub-style logo).
pub fn hub_badge_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(ACCENT)),
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
    container::Style {
        background: Some(iced::Background::Color(BG_DEEP)),
        border: Border {
            color: BORDER_SUBTLE,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

/// Tab bar background.
pub fn tab_bar_style(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG_SURFACE)),
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
        background: Some(iced::Background::Color(ACCENT)),
        ..Default::default()
    }
}

/// Table header row: elevated background + bottom hairline.
pub fn table_header_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG_ELEVATED)),
        border: Border {
            color: BORDER_SUBTLE,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

/// Gold accent separator line (1px high container).
pub fn accent_separator(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BORDER_ACCENT)),
        ..Default::default()
    }
}

/// Section card: wraps charts, stat tables, etc. with subtle border + roundness.
pub fn section_card_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG_SURFACE)),
        border: Border {
            color: BORDER_SUBTLE,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

/// Table data row with alternating colour.
/// `is_suspect` overrides the alternating tint.
/// `is_flash_on` overrides everything (highlight blink).
pub fn table_row_bg(
    index: usize,
    is_suspect: bool,
    is_flash_on: bool,
) -> impl Fn(&Theme) -> container::Style {
    move |theme: &Theme| {
        let bg = if is_flash_on {
            ROW_FLASH
        } else if is_suspect {
            ROW_SUSPECT
        } else if index % 2 == 1 {
            ROW_ALT
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
        background: Some(iced::Background::Color(CELL_OUTLIER)),
        ..Default::default()
    }
}

/// Text colour for outlier values.
pub const OUTLIER_TEXT: Color = Color {
    r: 0.95,
    g: 0.30,
    b: 0.30,
    a: 1.0,
};

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
    container::Style {
        background: Some(iced::Background::Color(BG_ELEVATED)),
        border: Border {
            color: BORDER_SUBTLE,
            width: 0.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

/// Tooltip: dark, elevated, rounded.
pub fn tooltip_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG_ELEVATED)),
        text_color: Some(TEXT_PRIMARY),
        border: Border {
            color: BORDER_SUBTLE,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    }
}

/// Full-screen overlay (decode-in-progress).
pub fn overlay_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(Color {
            a: 0.70,
            ..BG_DEEP
        })),
        text_color: Some(TEXT_PRIMARY),
        ..Default::default()
    }
}

/// Inline editor container (note / metric edit).
pub fn editor_container_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG_SURFACE)),
        border: Border {
            color: BORDER_SUBTLE,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

/// Dialog box (export-delete prompt, etc.).
pub fn dialog_box_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG_ELEVATED)),
        border: Border {
            color: BORDER_SUBTLE,
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
            ..BG_DEEP
        })),
        ..Default::default()
    }
}



/// Selected file item in analysis page job list.
pub fn selected_job_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(ACCENT_DIM)),
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
    s.border.color = DANGER;
    s.border.width = 2.0;
    s
}

/// Undo toast container.
pub fn undo_toast_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG_ELEVATED)),
        border: Border {
            color: BORDER_SUBTLE,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

/// Cache warning banner.
pub fn cache_warning_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG_SURFACE)),
        border: Border {
            color: Color { a: 0.40, ..WARNING },
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
    let base = button::Style {
        background: Some(iced::Background::Color(BG_ELEVATED)),
        text_color: TEXT_PRIMARY,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: BTN_RADIUS.into(),
        },
        ..Default::default()
    };
    match status {
        button::Status::Hovered => button::Style {
            background: Some(iced::Background::Color(BG_SURFACE)),
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(iced::Background::Color(BORDER_SUBTLE)),
            ..base
        },
        _ => base,
    }
}

/// Danger / destructive button: orange background, black text, generous radius.
/// Inspired by PornHub's prominent action buttons.
pub fn danger_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        background: Some(iced::Background::Color(ACCENT)),
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
            background: Some(iced::Background::Color(color!(0xFFAA22))),
            ..base
        },
        button::Status::Pressed => button::Style {
            background: Some(iced::Background::Color(color!(0xCC7700))),
            ..base
        },
        _ => base,
    }
}

// ────────────────────────  Plotters colours  ────────────────────────
// Wrappers for the `plotters` crate which uses its own `RGBColor`.

pub mod chart {
    use plotters::prelude::*;

    pub const BG: RGBColor = RGBColor(0x0d, 0x0d, 0x0d);
    pub const AXIS: RGBColor = RGBColor(0x44, 0x44, 0x44);
    pub const LABEL: RGBColor = RGBColor(0xcc, 0xcc, 0xcc);
    pub const TICK: RGBColor = RGBColor(0x88, 0x88, 0x88);
    pub const NORMAL_LINE: RGBAColor = RGBAColor(150, 180, 220, 0.22);
    pub const SUSPECT_LINE: RGBAColor = RGBAColor(244, 67, 54, 0.75);
    pub const HIGHLIGHT_LINE: RGBAColor = RGBAColor(255, 153, 0, 1.0);
    pub const TOOLTIP_BG: RGBColor = RGBColor(0x27, 0x27, 0x27);
    pub const TOOLTIP_BORDER: RGBColor = RGBColor(0x44, 0x44, 0x44);
    pub const TOOLTIP_HEADER: RGBColor = RGBColor(0xff, 0x99, 0x00);
    pub const LEGEND_NORMAL: RGBColor = RGBColor(150, 180, 220);
    pub const LEGEND_SUSPECT: RGBColor = RGBColor(244, 67, 54);
}
