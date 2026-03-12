//! Material Symbols icon font integration.
//!
//! Provides a subset of **Material Symbols Outlined** as an embedded font,
//! with constants for each icon codepoint used in the app.
//!
//! ## Adding new icons
//!
//! 1. Look up the codepoint using `pyftsubset` or fontTools against the full
//!    `MaterialSymbolsOutlined[…].ttf` variable font.
//! 2. Add a `pub(crate) const ICON_…` constant below.
//! 3. Regenerate the font subset — see `assets/README.md` for the full command.

use iced::widget::text;
use iced::{Element, Font};

/// The Material Symbols icon font, loaded from the embedded subset.
pub(crate) const ICON_FONT: Font = Font::with_name("Material Symbols Outlined");

/// Font bytes for registration via `iced::Settings::fonts`.
pub(crate) const ICON_FONT_BYTES: &[u8] = include_bytes!("../assets/material-symbols.ttf");

// ──────────────────────── Icon Codepoints ────────────────────────
// Codepoints are from Material Symbols Outlined variable font.
// Verified against /tmp/MaterialSymbolsOutlined.ttf using fontTools.
//
// ⚠ Do NOT use codepoints from the legacy "Material Icons" font —
//   they share glyph names but have DIFFERENT codepoints.

// ── Navigation & Actions ──
pub(crate) const ICON_CLOSE: &str = "\u{E14C}";            // close
pub(crate) const ICON_CHEVRON_LEFT: &str = "\u{E408}";     // chevron_left
pub(crate) const ICON_CHEVRON_RIGHT: &str = "\u{E409}";    // chevron_right
pub(crate) const ICON_ARROW_UPWARD: &str = "\u{E5D8}";     // arrow_upward
pub(crate) const ICON_ARROW_DOWNWARD: &str = "\u{E5DB}";   // arrow_downward
pub(crate) const ICON_UNFOLD_MORE: &str = "\u{E5D7}";      // unfold_more (sortable hint)
pub(crate) const ICON_MORE_VERT: &str = "\u{E5D4}";        // more_vert
pub(crate) const ICON_SEARCH: &str = "\u{E8B6}";           // search
pub(crate) const ICON_UNDO: &str = "\u{E166}";             // undo
pub(crate) const ICON_SELECT_ALL: &str = "\u{E162}";       // select_all

// ── CRUD & File operations ──
pub(crate) const ICON_EDIT: &str = "\u{E150}";             // edit
pub(crate) const ICON_DELETE: &str = "\u{E872}";           // delete
pub(crate) const ICON_DOWNLOAD: &str = "\u{E171}";         // download
pub(crate) const ICON_DESCRIPTION: &str = "\u{E873}";      // description (file/document)
pub(crate) const ICON_FOLDER: &str = "\u{E2C7}";           // folder

// ── Status & Feedback ──
pub(crate) const ICON_CHECK_CIRCLE: &str = "\u{E86C}";     // check_circle
pub(crate) const ICON_CANCEL: &str = "\u{E5C9}";           // cancel
pub(crate) const ICON_WARNING: &str = "\u{E002}";          // warning
pub(crate) const ICON_ERROR: &str = "\u{E000}";            // error
pub(crate) const ICON_INFO: &str = "\u{E88E}";             // info
pub(crate) const ICON_HELP: &str = "\u{E887}";             // help

// ── Session / History ──
pub(crate) const ICON_STAR: &str = "\u{E838}";             // star
pub(crate) const ICON_HISTORY: &str = "\u{E28E}";          // history
pub(crate) const ICON_CLEANING: &str = "\u{F0FF}";         // cleaning_services

// ── Notes & Comments ──
pub(crate) const ICON_COMMENT: &str = "\u{E0B9}";          // comment (no note)
pub(crate) const ICON_COMMENT_FILLED: &str = "\u{F1FC}";   // sticky_note_2 (has note)

// ── Processing / Progress ──
pub(crate) const ICON_HOURGLASS: &str = "\u{E88B}";        // hourglass_empty (queued)
pub(crate) const ICON_HOURGLASS_TOP: &str = "\u{EA5B}";    // hourglass_top (decoding)
pub(crate) const ICON_SYNC: &str = "\u{E627}";             // sync (processing)

// ── Statistics & Charts ──
pub(crate) const ICON_BAR_CHART: &str = "\u{E26B}";        // bar_chart
pub(crate) const ICON_MONITORING: &str = "\u{F190}";        // monitoring (dashboard card)
pub(crate) const ICON_QUERY_STATS: &str = "\u{E4FC}";      // query_stats (outlier rate)
pub(crate) const ICON_PERCENT: &str = "\u{EB58}";          // percent
pub(crate) const ICON_CODE: &str = "\u{E86F}";             // code (GitHub link)

/// Helper: create an icon text element with the given codepoint and size.
pub(crate) fn icon<'a, Message: 'a>(codepoint: &'a str, size: f32) -> Element<'a, Message> {
    text(codepoint)
        .font(ICON_FONT)
        .size(size)
        .into()
}
