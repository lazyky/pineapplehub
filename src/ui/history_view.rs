//! History page view: Sessions Sidebar + Tab Bar + Main Panel.
//!
//! Uses a two-column layout (sidebar + panel) with a tab bar for panel switching.

use std::collections::{HashMap, HashSet};
use crate::SortColumn;
use crate::history::stats::MetricColumn;

use iced::{
    Element, Length,
    widget::{
        button, checkbox, column, container, row, scrollable, space, text,
        text_input, tooltip,
    },
};

use crate::history::model::{AnalysisRecord, SessionSummary, StoredMetrics};
use crate::history::store::{CacheWarningLevel, MAX_RECORDS};
use crate::icons;
use crate::theme;
use crate::Message;

// ──────────────────────── Tab Bar ────────────────────────

/// Render tab bar at the top of the main panel for switching Records/Statistics.
pub(crate) fn view_tab_bar<'a>(
    current_panel: &HistoryPanel,
) -> Element<'a, Message> {
    let tab = |label: &'static str, icon: &'static str, panel: HistoryPanel| -> Element<'_, Message> {
        let is_active = *current_panel == panel;
        let tab_content = row![
            text(icon).font(icons::ICON_FONT).size(14),
            text(label).size(13),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center);

        let tab_btn = button(tab_content)
            .on_press(Message::HistorySetPanel(panel))
            .padding([6, 16])
            .style(if is_active { button::text } else { button::secondary });

        if is_active {
            // Active tab: accent-colored bottom border
            column![
                tab_btn,
                container(space::horizontal().width(0))
                    .width(Length::Fill)
                    .height(3)
                    .style(crate::theme::active_tab_underline),
            ]
            .width(Length::Shrink)
            .into()
        } else {
            column![
                tab_btn,
                space::vertical().height(3),
            ]
            .width(Length::Shrink)
            .into()
        }
    };

    container(
        row![
            tab(
                "Records",
                icons::ICON_DESCRIPTION,
                HistoryPanel::Records,
            ),
            tab(
                "Statistics",
                icons::ICON_BAR_CHART,
                HistoryPanel::Statistics,
            ),
            space::horizontal().width(Length::Fill),
        ]
        .spacing(4)
        .align_y(iced::Alignment::End),
    )
    .style(crate::theme::tab_bar_style)
    .padding([0, 8])
    .width(Length::Fill)
    .into()
}

/// Compose the main content area: sidebar toggle + tab bar + panel content.
pub(crate) fn view_main_content<'a>(
    panel: &HistoryPanel,
    records: &'a [AnalysisRecord],
    selected_count: usize,
    editing_note: &'a Option<(String, String)>,
    editing_metric: &'a Option<(String, StoredMetrics)>,
    editing_metric_text: &'a [String; 4],
    record_filter: crate::RecordFilter,
    search_query: &'a str,
    sort_column: Option<crate::SortColumn>,
    sort_ascending: bool,
    outlier_cells: &'a std::collections::HashMap<String, HashSet<MetricColumn>>,
    column_stats_map: &'a std::collections::HashMap<MetricColumn, crate::history::stats::ColumnStats>,
    parallel_coords_chart: &'a super::parallel_coords::ParallelCoordsChart,
    highlight_record_id: &'a Option<String>,
    highlight_ticks: u8,
    sidebar_open: bool,
) -> Element<'a, Message> {
    let tab_bar = view_tab_bar(panel);

    // Sidebar toggle chevron
    let toggle_icon = if sidebar_open {
        icons::ICON_CHEVRON_LEFT
    } else {
        icons::ICON_CHEVRON_RIGHT
    };
    let toggle_btn: Element<'_, Message> = tooltip(
        button(
            text(toggle_icon).font(icons::ICON_FONT).size(16).center(),
        )
        .width(28)
        .height(28)
        .style(theme::text_button_style)
        .on_press(Message::ToggleSidebar),
        if sidebar_open {
            "Hide sidebar"
        } else {
            "Show sidebar"
        },
        tooltip::Position::Bottom,
    )
    .style(tooltip_style)
    .into();

    let top_row = row![toggle_btn, tab_bar]
        .spacing(0)
        .align_y(iced::Alignment::Center);

    let content: Element<'_, Message> = match panel {
        HistoryPanel::Records => view_records_panel(
            records,
            selected_count,
            editing_note,
            editing_metric,
            editing_metric_text,
            record_filter,
            search_query,
            sort_column,
            sort_ascending,
            outlier_cells,
            highlight_record_id,
            highlight_ticks,
        ),
        HistoryPanel::Statistics => {
            view_statistics_panel(
                records,
                selected_count,
                parallel_coords_chart,
                column_stats_map,
                outlier_cells,
            )
        }
    };

    column![top_row, content]
        .spacing(0)
        .height(Length::Fill)
        .width(Length::Fill)
        .into()
}

// ──────────────────────── Sessions Sidebar ────────────────────────

/// Render the sessions sidebar (list of sessions with checkboxes).
pub(crate) fn view_sessions_sidebar<'a>(
    sessions: &'a [SessionSummary],
    selected: &'a HashSet<String>,
    cache_warning: &'a Option<CacheWarningLevel>,
    delete_confirm: &'a Option<(Vec<String>, u32)>,
    clear_all_confirm: bool,
    editing_session_name: &'a Option<(String, String)>,
) -> Element<'a, Message> {
    let mut col = column![].spacing(6).padding(12).width(Length::Fill);

    // Cache warning banner
    if let Some(warning) = cache_warning {
        col = col.push(view_cache_warning(warning));
    }

    // Toolbar
    let all_selected = !sessions.is_empty()
        && sessions
            .iter()
            .all(|s| selected.contains(&s.session_id));

    let mut toolbar = row![]
        .spacing(8)
        .padding(4);
    toolbar = toolbar.push(
        checkbox(all_selected)
            .label("All")
            .on_toggle(Message::ToggleAllSessions),
    );
    if !selected.is_empty() {
        toolbar = toolbar.push(
            tooltip(
                button(text(icons::ICON_DELETE).font(icons::ICON_FONT).size(14))
                    .on_press(Message::DeleteSelectedSessions)
                    .style(theme::danger_button_style),
                "Delete selected",
                tooltip::Position::Bottom,
            ).style(tooltip_style),
        );
        toolbar = toolbar.push(
            tooltip(
                button(text(icons::ICON_DOWNLOAD).font(icons::ICON_FONT).size(14))
                    .on_press(Message::ExportSelectedSessions)
                    .style(theme::secondary_button_style),
                "Export selected",
                tooltip::Position::Bottom,
            ).style(tooltip_style),
        );
    }
    col = col.push(toolbar);

    // Delete confirmation banner
    if let Some((sids, record_count)) = delete_confirm {
        let session_count = sids.len();
        col = col.push(
            container(
                column![
                    text(format!(
                        "Delete {session_count} session(s) ({record_count} records)?"
                    ))
                    .size(13),
                    row![
                        button(
                            row![
                                text(icons::ICON_DELETE).font(icons::ICON_FONT).size(14),
                                text(" Confirm").size(13),
                            ]
                            .align_y(iced::Alignment::Center),
                        )
                            .on_press(Message::ConfirmDelete)
                            .style(theme::danger_button_style),
                        button(
                            row![
                                text(icons::ICON_CLOSE).font(icons::ICON_FONT).size(14),
                                text(" Cancel").size(13),
                            ]
                            .align_y(iced::Alignment::Center),
                        )
                            .on_press(Message::CancelDelete)
                            .style(theme::secondary_button_style),
                    ]
                    .spacing(8),
                ]
                .spacing(8)
                .padding(8),
            )
            .style(container::bordered_box)
            .width(Length::Fill),
        );
    }

    // Session list
    if sessions.is_empty() {
        col = col.push(
            container(text("No history yet.\nRun a batch analysis first.").size(14))
                .padding(20)
                .center(Length::Fill),
        );
    } else {
        let session_list = column(sessions.iter().map(|session| {
            let is_selected = selected.contains(&session.session_id);
            let timestamp = format_timestamp(session.timestamp);

            let mut info = format!("{} files", session.total_count);
            if session.failed_count > 0 {
                info += &format!(" · {}×", session.failed_count);
            }

            // Build info line: plain text + optional warning icon with suspect count
            let info_el: Element<'_, Message> = if session.suspect_count > 0 {
                row![
                    text(info).size(12),
                    text(" · ").size(12),
                    text(icons::ICON_WARNING).font(icons::ICON_FONT).size(11),
                    text(format!("{}", session.suspect_count)).size(12),
                ]
                .align_y(iced::Alignment::Center)
                .into()
            } else {
                text(info).size(12).into()
            };

            let (star_label, star_style): (&str, fn(&iced::Theme, button::Status) -> button::Style) =
                if session.starred {
                    (icons::ICON_STAR, button::primary)
                } else {
                    (icons::ICON_STAR, button::secondary)
                };

            // Check if we are editing this session's name
            let is_renaming = editing_session_name
                .as_ref()
                .map_or(false, |(sid, _)| sid == &session.session_id);

            let name_area: Element<'_, Message> = if is_renaming {
                // Inline text_input for renaming
                let current_val = editing_session_name
                    .as_ref()
                    .map_or("", |(_, v)| v.as_str());
                text_input("Session name…", current_val)
                    .on_input(|v| Message::RenameSessionInput(v))
                    .on_submit(Message::SubmitSessionRename)
                    .size(13)
                    .width(Length::Fill)
                    .into()
            } else {
                // Display name or timestamp, with double-click to rename
                let display_name = session.name.clone().unwrap_or_else(|| timestamp.clone());
                let sid = session.session_id.clone();
                let sid_click = session.session_id.clone();

                let name_content: Element<'_, Message> = if session.name.is_some() {
                    // Has custom name: show name + timestamp below in small text
                    column![
                        text(display_name).size(13),
                        text(timestamp).size(10).color(iced::Color::from_rgba(0.6, 0.6, 0.6, 1.0)),
                        info_el,
                    ]
                    .spacing(2)
                    .width(Length::Fill)
                    .into()
                } else {
                    // Default: show timestamp + info
                    column![
                        text(display_name).size(13),
                        info_el,
                    ]
                    .spacing(2)
                    .width(Length::Fill)
                    .into()
                };

                tooltip(
                    iced::widget::MouseArea::new(name_content)
                        .on_press(Message::ToggleSessionSelected(sid_click, !is_selected))
                        .on_double_click(Message::StartRenameSession(sid)),
                    container(
                        row![
                            text(icons::ICON_EDIT).font(icons::ICON_FONT).size(12),
                            text(" Double-click to rename").size(12),
                        ].align_y(iced::Alignment::Center),
                    ),
                    tooltip::Position::Bottom,
                )
                .style(tooltip_style)
                .into()
            };

            row![
                checkbox(is_selected)
                    .on_toggle({
                        let sid = session.session_id.clone();
                        move |checked| Message::ToggleSessionSelected(sid.clone(), checked)
                    }),
                tooltip(
                    button(text(star_label).font(icons::ICON_FONT).size(16))
                        .on_press(Message::ToggleSessionStar(
                            session.session_id.clone(),
                            !session.starred,
                        ))
                        .style(star_style)
                        .padding([1, 4]),
                    if session.starred { "Unstar" } else { "Star" },
                    tooltip::Position::Bottom,
                ).style(tooltip_style),
                name_area,
            ]
            .spacing(4)
            .padding(4)
            .into()
        }))
        .spacing(2);

        col = col.push(scrollable(session_list).height(Length::Fill));
    }

    // Bottom buttons
    let mut bottom = column![].spacing(4).padding([4, 8]);

    bottom = bottom.push(
        button(
            row![text(icons::ICON_CLEANING).font(icons::ICON_FONT).size(14), text(" Quick Cleanup").size(12)]
                .align_y(iced::Alignment::Center),
        )
            .on_press(Message::QuickCleanup)
            .style(theme::cleanup_button_style)
            .width(Length::Fill),
    );

    // Clear All confirmation or button
    if clear_all_confirm {
        bottom = bottom.push(
            container(
                column![
                    text("Permanently delete ALL history (including starred)?").size(12),
                    row![
                        button(
                            row![
                                text(icons::ICON_DELETE).font(icons::ICON_FONT).size(14),
                                text(" Clear All").size(12),
                            ]
                            .align_y(iced::Alignment::Center),
                        )
                            .on_press(Message::ConfirmClearAll)
                            .style(theme::danger_button_style),
                        button(
                            row![
                                text(icons::ICON_CLOSE).font(icons::ICON_FONT).size(14),
                                text(" Cancel").size(12),
                            ]
                            .align_y(iced::Alignment::Center),
                        )
                            .on_press(Message::CancelClearAll)
                            .style(theme::secondary_button_style),
                    ]
                    .spacing(8),
                ]
                .spacing(6)
                .padding(8),
            )
            .style(container::bordered_box)
            .width(Length::Fill),
        );
    } else if !sessions.is_empty() {
        bottom = bottom.push(
            button(
                row![
                    text(icons::ICON_DELETE).font(icons::ICON_FONT).size(14),
                    text(" Clear All History").size(12),
                ]
                .align_y(iced::Alignment::Center),
            )
                .on_press(Message::ClearAllHistory)
                .style(theme::danger_button_style)
                .width(Length::Fill),
        );
    }

    col = col.push(bottom);

    col.into()
}

// ──────────────────────── Records Panel ────────────────────────

/// Render the records table for selected sessions.
pub(crate) fn view_records_panel<'a>(
    records: &'a [AnalysisRecord],
    selected_sessions_count: usize,
    editing_note: &'a Option<(String, String)>,
    editing_metric: &'a Option<(String, StoredMetrics)>,
    editing_metric_text: &'a [String; 4],
    record_filter: crate::RecordFilter,
    search_query: &'a str,
    sort_column: Option<SortColumn>,
    sort_ascending: bool,
    outlier_cells: &'a HashMap<String, HashSet<MetricColumn>>,
    highlight_record_id: &'a Option<String>,
    highlight_ticks: u8,
) -> Element<'a, Message> {

    let mut col = column![].spacing(8).padding(8);

    if records.is_empty() {
        col = col.push(
            container(text("← Select sessions to view records").size(14))
                .padding(40)
                .center(Length::Fill),
        );
        return col.into();
    }

    // Filter records by filename
    let query_lower = search_query.to_lowercase();
    let mut filtered: Vec<&AnalysisRecord> = if query_lower.is_empty() {
        records.iter().collect()
    } else {
        records
            .iter()
            .filter(|r| r.filename.to_lowercase().contains(&query_lower))
            .collect()
    };

    // Apply quick filter
    use crate::RecordFilter;
    match record_filter {
        RecordFilter::All => {}
        RecordFilter::SuspectsOnly => { filtered.retain(|r| r.suspect); }
        RecordFilter::NormalOnly => { filtered.retain(|r| !r.suspect); }
        RecordFilter::HasNote => { filtered.retain(|r| !r.note.is_empty()); }
    }

    // Sort
    if let Some(sc) = sort_column {
        filtered.sort_by(|a, b| {
            let am = &a.metrics;
            let bm = &b.metrics;
            let cmp = match sc {
                SortColumn::Filename => a.filename.cmp(&b.filename),
                SortColumn::Height => am.major_length.partial_cmp(&bm.major_length).unwrap_or(std::cmp::Ordering::Equal),
                SortColumn::Width => am.minor_length.partial_cmp(&bm.minor_length).unwrap_or(std::cmp::Ordering::Equal),
                SortColumn::Volume => am.volume.partial_cmp(&bm.volume).unwrap_or(std::cmp::Ordering::Equal),
                SortColumn::Aeq => am.a_eq.unwrap_or(0.0).partial_cmp(&bm.a_eq.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal),
                SortColumn::Beq => am.b_eq.unwrap_or(0.0).partial_cmp(&bm.b_eq.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal),
                SortColumn::SurfaceArea => am.surface_area.unwrap_or(0.0).partial_cmp(&bm.surface_area.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal),
                SortColumn::NTotal => am.n_total.unwrap_or(0).cmp(&bm.n_total.unwrap_or(0)),
            };
            if sort_ascending { cmp } else { cmp.reverse() }
        });
    }

    // Header with search
    col = col.push(
        row![
            text(format!(
                "Showing {} sessions ({} records{})",
                selected_sessions_count,
                filtered.len(),
                if filtered.len() != records.len() {
                    format!(" / {} total", records.len())
                } else {
                    String::new()
                },
            ))
            .size(14),
            space::horizontal().width(Length::Fill),
            row![
                text(icons::ICON_SEARCH).font(icons::ICON_FONT).size(14),
                text_input("Search filename...", search_query)
                    .on_input(Message::SearchQueryChanged)
                    .width(180),
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center),
            button(
                row![text(icons::ICON_DOWNLOAD).font(icons::ICON_FONT).size(14), text(" Export CSV").size(12)]
                    .align_y(iced::Alignment::Center),
            )
                .on_press(Message::ExportSelectedSessions)
                .style(theme::secondary_button_style),
        ]
        .spacing(8),
    );

    // Quick filter buttons
    let filter_btn = |label: &'static str, f: RecordFilter| -> Element<'_, Message> {
        let style = if record_filter == f { button::primary } else { button::secondary };
        button(text(label).size(11))
            .on_press(Message::ToggleRecordFilter(f))
            .style(style)
            .padding([2, 8])
            .into()
    };
    col = col.push(
        row![
            text("Filter:").size(12),
            filter_btn("All", RecordFilter::All),
            filter_btn("Suspects", RecordFilter::SuspectsOnly),
            filter_btn("Normal", RecordFilter::NormalOnly),
            filter_btn("Noted", RecordFilter::HasNote),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center),
    );

    // Sortable header helper — each column gets a tooltip describing its meaning
    let sort_hdr = |label: &'static str, tip: &'static str, sc: SortColumn, portion: u16| -> Element<'_, Message> {
        let icon_str = if sort_column == Some(sc) {
            if sort_ascending { icons::ICON_ARROW_UPWARD } else { icons::ICON_ARROW_DOWNWARD }
        } else {
            icons::ICON_UNFOLD_MORE
        };
        tooltip(
            button(
                row![
                    text(label).size(13),
                    text(icon_str).font(icons::ICON_FONT).size(12),
                ]
                .spacing(2)
                .align_y(iced::Alignment::Center),
            )
            .on_press(Message::SortBy(sc))
            .style(theme::text_button_style)
            .padding([2, 4])
            .width(Length::FillPortion(portion)),
            tip,
            tooltip::Position::Bottom,
        )
        .style(tooltip_style)
        .into()
    };

    // Table header
    let header = row![
        sort_hdr("File", "Source image filename", SortColumn::Filename, 3),
        sort_hdr("H", MetricColumn::Height.description(), SortColumn::Height, 1),
        sort_hdr("D", MetricColumn::Width.description(), SortColumn::Width, 1),
        sort_hdr("V", MetricColumn::Volume.description(), SortColumn::Volume, 1),
        sort_hdr("a", MetricColumn::Aeq.description(), SortColumn::Aeq, 1),
        sort_hdr("b", MetricColumn::Beq.description(), SortColumn::Beq, 1),
        sort_hdr("S", MetricColumn::SurfaceArea.description(), SortColumn::SurfaceArea, 1),
        sort_hdr("Nf", MetricColumn::NTotal.description(), SortColumn::NTotal, 1),
        text("Actions").size(13).width(Length::FillPortion(2)),
    ]
    .spacing(6);
    col = col.push(header);

    // Table rows
    let mut row_index: usize = 0;
    let rows = column(
        filtered
            .into_iter()
            .flat_map(|record| {
                let mut elements: Vec<Element<'_, Message>> = Vec::new();

                // Main row
                let m = &record.metrics;
                let record_outliers = outlier_cells.get(&record.id);
                let is_suspect = record.suspect;
                // Flash highlight: on odd ticks, show accent background
                let is_flash_on = highlight_record_id.as_deref() == Some(&record.id)
                    && highlight_ticks % 2 == 1;
                let idx = row_index;
                row_index += 1;
                let row_bg = crate::theme::table_row_bg(idx, is_suspect, is_flash_on);

                let filename_cell: Element<'_, Message> = if m.manually_edited {
                    row![
                        text(&record.filename).size(13),
                        text(icons::ICON_EDIT).font(icons::ICON_FONT).size(11),
                    ]
                    .spacing(2)
                    .width(Length::FillPortion(3))
                    .into()
                } else {
                    text(&record.filename)
                        .size(13)
                        .width(Length::FillPortion(3))
                        .into()
                };

                // Helper for a metric cell with possible outlier highlighting
                let outlier_set = record_outliers.cloned().unwrap_or_default();

                let metric_cell = |value: String, col: MetricColumn, portion: u16| -> Element<'_, Message> {
                    let is_outlier = outlier_set.contains(&col);
                    let txt = text(value).size(13);
                    let txt = if is_outlier {
                        txt.color(crate::theme::outlier_text())
                    } else {
                        txt
                    };
                    if is_outlier {
                        container(txt)
                            .style(crate::theme::outlier_cell_style)
                            .width(Length::FillPortion(portion))
                            .into()
                    } else {
                        container(txt)
                            .width(Length::FillPortion(portion))
                            .into()
                    }
                };

                let row_content = container(
                    row![
                        filename_cell,
                        metric_cell(format!("{:.1}", m.major_length), MetricColumn::Height, 1),
                        metric_cell(format!("{:.1}", m.minor_length), MetricColumn::Width, 1),
                        metric_cell(format!("{:.0}", m.volume), MetricColumn::Volume, 1),
                        metric_cell(m.a_eq.map_or("-".into(), |v| format!("{v:.1}")), MetricColumn::Aeq, 1),
                        metric_cell(m.b_eq.map_or("-".into(), |v| format!("{v:.1}")), MetricColumn::Beq, 1),
                        metric_cell(m.surface_area.map_or("-".into(), |v| format!("{v:.0}")), MetricColumn::SurfaceArea, 1),
                        metric_cell(m.n_total.map_or("-".into(), |v| format!("{v}")), MetricColumn::NTotal, 1),
                        container(view_record_actions(record))
                            .width(Length::FillPortion(2)),
                    ]
                    .spacing(6)
                    .align_y(iced::Alignment::Center),
                )
                .style(row_bg)
                .padding([4, 6]);

                elements.push(row_content.into());

                // Inline note editor (if this record is being edited)
                if let Some((edit_id, note_text)) = editing_note {
                    if edit_id == &record.id {
                        elements.push(view_note_editor(edit_id, note_text));
                    }
                }

                // Inline metric editor
                if let Some((edit_id, _edit_metrics)) = editing_metric {
                    if edit_id == &record.id {
                        elements.push(view_metric_editor(edit_id, editing_metric_text));
                    }
                }

                elements
            })
            .collect::<Vec<_>>(),
    )
    .spacing(4);

    col = col.push(scrollable(rows).height(Length::Fill));

    col.into()
}

/// Action icons for a single record row.
fn view_record_actions(record: &AnalysisRecord) -> Element<'_, Message> {
    // Icon represents the ACTION: ⚠ to flag, ✓ to clear
    let suspect_icon = if record.suspect { icons::ICON_CHECK_CIRCLE } else { icons::ICON_WARNING };
    let suspect_tip = if record.suspect { "Mark as verified" } else { "Mark as suspect" };

    let has_note = !record.note.is_empty();
    let note_icon = if has_note { icons::ICON_COMMENT_FILLED } else { icons::ICON_COMMENT };
    let note_tip = if has_note { "Edit note" } else { "Add note" };

    row![
        tooltip(
            button(text(suspect_icon).font(icons::ICON_FONT).size(16))
                .on_press(Message::ToggleSuspect(
                    record.id.clone(),
                    !record.suspect,
                ))
                .style(theme::text_button_style)
                .padding(2),
            suspect_tip,
            tooltip::Position::Bottom,
        ).style(tooltip_style),
        tooltip(
            button(text(note_icon).font(icons::ICON_FONT).size(16))
                .on_press(Message::OpenNoteEditor(record.id.clone()))
                .style(theme::text_button_style)
                .padding(2),
            note_tip,
            tooltip::Position::Bottom,
        ).style(tooltip_style),
        tooltip(
            button(text(icons::ICON_EDIT).font(icons::ICON_FONT).size(16))
                .on_press(Message::OpenMetricEditor(record.id.clone()))
                .style(theme::text_button_style)
                .padding(2),
            "Edit metrics",
            tooltip::Position::Bottom,
        ).style(tooltip_style),
    ]
    .spacing(2)
    .into()
}

fn view_note_editor<'a>(record_id: &str, note_text: &str) -> Element<'a, Message> {
    let rid_input = record_id.to_string();

    container(
        row![
            text_input("Enter note...", note_text)
                .on_input(move |val| Message::NoteInputChanged(rid_input.clone(), val))
                .on_submit(Message::SubmitCurrentNote)
                .width(Length::Fill),
            tooltip(
                button(
                    text(icons::ICON_CHECK_CIRCLE).font(icons::ICON_FONT).size(16)
                )
                    .on_press(Message::SubmitCurrentNote)
                    .style(theme::primary_button_style)
                    .padding(4),
                "Save",
                tooltip::Position::Bottom,
            ).style(tooltip_style),
            tooltip(
                button(
                    text(icons::ICON_CLOSE).font(icons::ICON_FONT).size(16)
                )
                    .on_press(Message::CancelEdit)
                    .style(theme::secondary_button_style)
                    .padding(4),
                "Cancel",
                tooltip::Position::Bottom,
            ).style(tooltip_style),
            tooltip(
                button(
                    text(icons::ICON_DELETE).font(icons::ICON_FONT).size(16)
                )
                    .on_press(Message::DeleteCurrentNote)
                    .style(theme::danger_button_style)
                    .padding(4),
                "Delete note",
                tooltip::Position::Bottom,
            ).style(tooltip_style),
        ]
        .spacing(4)
        .padding(4),
    )
    .style(container::bordered_box)
    .into()
}

fn view_metric_editor<'a>(_record_id: &str, texts: &[String; 4]) -> Element<'a, Message> {
    let fields: [(&str, usize); 4] = [
        ("H (mm):", 0),
        ("D (mm):", 1),
        ("a (mm):", 2),
        ("b (mm):", 3),
    ];

    let mut col = column![].spacing(4).padding(8);

    for &(label, idx) in &fields {
        let current_text = texts[idx].clone();
        let is_valid = current_text.is_empty() || current_text.parse::<f32>().is_ok();
        let mut input = text_input("", &current_text)
            .on_input(move |val| Message::MetricInputChanged(idx, val))
            .on_submit(Message::SubmitCurrentMetric)
            .width(120);
        if !is_valid {
            input = input.style(crate::theme::validation_error_input);
        }
        col = col.push(
            row![
                text(label).size(13).width(100),
                input,
            ]
            .spacing(4),
        );
    }

    // Check if all required fields are valid (H and D must parse; a and b may be empty)
    let can_save = texts[0].parse::<f32>().is_ok()
        && texts[1].parse::<f32>().is_ok()
        && (texts[2].is_empty() || texts[2].parse::<f32>().is_ok())
        && (texts[3].is_empty() || texts[3].parse::<f32>().is_ok());

    let mut save_btn = button(
        text(icons::ICON_CHECK_CIRCLE).font(icons::ICON_FONT).size(16)
    )
    .style(theme::primary_button_style)
    .padding(4);
    if can_save {
        save_btn = save_btn.on_press(Message::SubmitCurrentMetric);
    }

    col = col.push(
        row![
            tooltip(save_btn, "Save", tooltip::Position::Bottom).style(tooltip_style),
            tooltip(
                button(text(icons::ICON_CLOSE).font(icons::ICON_FONT).size(16))
                    .on_press(Message::CancelEdit)
                    .style(theme::secondary_button_style)
                    .padding(4),
                "Cancel",
                tooltip::Position::Bottom,
            ).style(tooltip_style),
            tooltip(
                button(text(icons::ICON_HISTORY).font(icons::ICON_FONT).size(16))
                    .on_press(Message::ResetCurrentMetric)
                    .style(theme::danger_button_style)
                    .padding(4),
                "Reset to original",
                tooltip::Position::Bottom,
            ).style(tooltip_style),
        ]
        .spacing(4),
    );

    container(col)
        .style(crate::theme::editor_container_style)
        .into()
}

// ──────────────────────── Statistics Panel ────────────────────────

pub(crate) fn view_statistics_panel<'a>(
    records: &'a [AnalysisRecord],
    selected_sessions_count: usize,
    chart: &'a super::parallel_coords::ParallelCoordsChart,
    column_stats: &'a std::collections::HashMap<MetricColumn, crate::history::stats::ColumnStats>,
    _outlier_cells: &'a std::collections::HashMap<String, HashSet<MetricColumn>>,
) -> Element<'a, Message> {
    use crate::history::stats::ColumnStats;

    let mut col = column![].spacing(12).padding(16);

    if selected_sessions_count == 0 || column_stats.is_empty() {
        return container(
            column![
                text(icons::ICON_BAR_CHART).font(icons::ICON_FONT).size(24),
                text(" Statistics Module").size(20),
                text(if selected_sessions_count > 0 {
                    "Loading statistics...".into()
                } else {
                    "Select sessions from the sidebar to analyze.".to_string()
                })
                .size(13),
            ]
            .spacing(8)
            .padding(40),
        )
        .center(Length::Fill)
        .into();
    }

    // ── Summary cards ──
    let total_samples = records.len();
    let suspect_count = records.iter().filter(|r| r.suspect).count();
    let outlier_rate = if total_samples > 0 {
        suspect_count as f64 / total_samples as f64 * 100.0
    } else {
        0.0
    };

    let summary_card = |icon: &'static str, label: &'static str, value: String,
                        color: iced::Color| -> Element<'_, Message> {
        container(
            column![
                row![
                    text(icon).font(icons::ICON_FONT).size(14),
                    text(label).size(11),
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center),
                text(value).size(20),
            ]
            .spacing(4)
            .align_x(iced::Alignment::Center),
        )
        .padding([12, 20])
        .width(Length::FillPortion(1))
        .style(crate::theme::summary_card_style(color))
        .into()
    };

    let rate_color = if outlier_rate > 10.0 {
        crate::theme::danger()
    } else if outlier_rate > 5.0 {
        crate::theme::warning()
    } else {
        crate::theme::success()
    };

    col = col.push(
        row![
            summary_card(
                icons::ICON_MONITORING,
                "Samples",
                format!("{}", total_samples),
                iced::Color::from_rgb(0.3, 0.6, 1.0),
            ),
            summary_card(
                icons::ICON_WARNING,
                "Suspects",
                format!("{}", suspect_count),
                if suspect_count > 0 {
                    iced::Color::from_rgb(1.0, 0.5, 0.3)
                } else {
                    iced::Color::from_rgb(0.3, 0.8, 0.5)
                },
            ),
            summary_card(
                icons::ICON_QUERY_STATS,
                "Outlier Rate",
                format!("{:.1}%", outlier_rate),
                rate_color,
            ),
            summary_card(
                icons::ICON_FOLDER,
                "Sessions",
                format!("{}", selected_sessions_count),
                iced::Color::from_rgb(0.5, 0.5, 0.8),
            ),
        ]
        .spacing(8),
    );

    // ── Descriptive Statistics Table ──
    col = col.push(
        row![
            text(icons::ICON_BAR_CHART).font(icons::ICON_FONT).size(18),
            text(" Descriptive Statistics").size(16),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center),
    );

    // Table header — each column gets a tooltip
    let hdr = |label: &'static str, tip: &'static str, portion: u16| -> Element<'_, Message> {
        tooltip(
            text(label)
                .size(12)
                .width(Length::FillPortion(portion)),
            tip,
            tooltip::Position::Bottom,
        )
        .style(tooltip_style)
        .into()
    };
    let header_row = container(
        row![
            hdr("Metric", "Measurement variable", 2),
            hdr("n", "Sample count", 1),
            hdr("Mean", "Arithmetic mean", 2),
            hdr("Median", "50th percentile", 2),
            hdr("SD", "Standard deviation", 2),
            hdr("CV%", "Coefficient of variation (SD/Mean × 100%)", 1),
            hdr("Min", "Minimum value", 2),
            hdr("Max", "Maximum value", 2),
        ]
        .spacing(4),
    )
    .style(crate::theme::stats_header_style)
    .padding([6, 8]);
    // Wrap descriptive statistics in a section card
    let mut stats_table = column![header_row].spacing(0);

    // One row per metric
    let stat_row = |mc: MetricColumn, stats: &ColumnStats, idx: usize| -> Element<'_, Message> {
        let cell = |value: String, portion: u16| -> Element<'_, Message> {
            text(value)
                .size(13)
                .width(Length::FillPortion(portion))
                .into()
        };
        // CV = SD / Mean × 100%, show "-" if mean ≈ 0
        let cv = if stats.mean.abs() > 1e-12 {
            format!("{:.1}", stats.sd / stats.mean.abs() * 100.0)
        } else {
            "-".to_string()
        };
        let row_bg = crate::theme::table_row_bg(idx, false, false);
        container(
            row![
                text(mc.label())
                    .size(13)
                    .width(Length::FillPortion(2)),
                cell(format!("{}", stats.n), 1),
                cell(format!("{:.2}", stats.mean), 2),
                cell(format!("{:.2}", stats.median), 2),
                cell(format!("{:.2}", stats.sd), 2),
                cell(cv, 1),
                cell(format!("{:.2}", stats.min), 2),
                cell(format!("{:.2}", stats.max), 2),
            ]
            .spacing(4),
        )
        .style(row_bg)
        .padding([4, 8])
        .into()
    };

    let mut stat_idx: usize = 0;
    for mc in MetricColumn::ALL {
        if let Some(stats) = column_stats.get(&mc) {
            stats_table = stats_table.push(stat_row(mc, stats, stat_idx));
            stat_idx += 1;
        }
    }

    col = col.push(
        container(stats_table)
            .style(crate::theme::section_card_style)
            .padding(4)
            .width(Length::Fill),
    );

    // Parallel Coordinates Chart
    if !chart.is_empty() {
        col = col.push(
            container(
                column![
                    row![
                        text(icons::ICON_BAR_CHART).font(icons::ICON_FONT).size(18),
                        text(" Parallel Coordinates").size(16),
                    ]
                    .spacing(4)
                    .align_y(iced::Alignment::Center),
                    chart.view(),
                ]
                .spacing(8)
                .padding(8),
            )
            .style(crate::theme::section_card_style)
            .width(Length::Fill),
        );
    }

    scrollable(col).height(Length::Fill).into()
}

// ──────────────────────── Cache Warning Banner ────────────────────────

fn view_cache_warning(warning: &CacheWarningLevel) -> Element<'_, Message> {
    let content: Element<'_, Message> = match warning {
        CacheWarningLevel::Ok => return space::horizontal().height(0).into(),
        CacheWarningLevel::Caution {
            current,
            cleanable_sessions,
        } => {
            row![
                text(icons::ICON_INFO).font(icons::ICON_FONT).size(14),
                text(format!(
                    " Cache {current}/{MAX_RECORDS} ({cleanable_sessions} session(s) cleanable)"
                ))
                .size(12),
                button(
                    row![text(icons::ICON_CLEANING).font(icons::ICON_FONT).size(12), text(" Cleanup").size(12)]
                        .align_y(iced::Alignment::Center)
                )
                    .on_press(Message::QuickCleanup)
                    .style(theme::secondary_button_style),
                button(text(icons::ICON_CLOSE).font(icons::ICON_FONT).size(12))
                    .on_press(Message::DismissCacheWarning)
                    .style(theme::text_button_style),
            ]
            .spacing(4)
            .into()
        }
        CacheWarningLevel::Warning {
            current,
            cleanable_sessions,
        } => {
            row![
                text(icons::ICON_WARNING).font(icons::ICON_FONT).size(14),
                text(format!(
                    " Cache nearly full: {current}/{MAX_RECORDS} ({cleanable_sessions} cleanable)"
                ))
                .size(12),
                button(
                    row![text(icons::ICON_CLEANING).font(icons::ICON_FONT).size(12), text(" Cleanup Now").size(12)]
                        .align_y(iced::Alignment::Center)
                )
                    .on_press(Message::QuickCleanup)
                    .style(theme::danger_button_style),
            ]
            .spacing(4)
            .into()
        }
        CacheWarningLevel::Full { current } => {
            column![
                row![
                    text(icons::ICON_WARNING).font(icons::ICON_FONT).size(14),
                    text(format!(" Cache full ({current}/{MAX_RECORDS})")).size(13),
                ].spacing(4),
                text("Cannot save new results. Please clean up.").size(12),
                row![
                    button(
                        row![text(icons::ICON_CLEANING).font(icons::ICON_FONT).size(12), text(" Quick Cleanup").size(12)]
                            .align_y(iced::Alignment::Center),
                    )
                        .on_press(Message::QuickCleanup)
                        .style(theme::danger_button_style),
                    button(
                        row![text(icons::ICON_HISTORY).font(icons::ICON_FONT).size(12), text(" Manage History").size(12)]
                            .align_y(iced::Alignment::Center),
                    )
                        .on_press(Message::NavigateTo(Page::History {
                            panel: HistoryPanel::Records,
                            sidebar_open: true,
                        }))
                        .style(theme::secondary_button_style),
                ]
                .spacing(4),
            ]
            .spacing(4)
            .into()
        }
    };

    container(content)
        .padding(8)
        .style(crate::theme::cache_warning_style)
        .width(Length::Fill)
        .into()
}

// ──────────────────────── Undo Toast ────────────────────────

/// Render the undo toast at the bottom of the screen.
pub(crate) fn view_undo_toast<'a>(
    message: &'a str,
    countdown_secs: u8,
) -> Element<'a, Message> {
    container(
        row![
            text(message).size(13),
            space::horizontal().width(Length::Fill),
            text(format!("({countdown_secs}s)")).size(12),
            button(text("Undo").size(12))
                .on_press(Message::UndoDelete)
                .style(theme::primary_button_style),
        ]
        .spacing(8)
        .padding(8)
        .align_y(iced::Alignment::Center),
    )
    .style(crate::theme::undo_toast_style)
    .width(Length::Fill)
    .into()
}
// ──────────────────────── Export-Delete Prompt ────────────────────────

/// Render a prompt asking whether to delete exported sessions.
pub(crate) fn view_export_delete_prompt<'a>() -> Element<'a, Message> {
    container(
        container(
            column![
                text("Export Complete").size(16),
                text("Delete the exported sessions?").size(13),
                space::vertical().height(8),
                row![
                    button(
                        row![
                            text(icons::ICON_DELETE).font(icons::ICON_FONT).size(14),
                            text(" Delete").size(13),
                        ]
                        .align_y(iced::Alignment::Center),
                    )
                        .on_press(Message::DeleteExportedSessions)
                        .style(theme::danger_button_style),
                    button(text("Keep").size(13))
                        .on_press(Message::DismissExportPrompt)
                        .style(theme::secondary_button_style),
                ]
                .spacing(8),
            ]
            .spacing(8)
            .padding(24)
            .align_x(iced::Alignment::Center),
        )
        .style(crate::theme::dialog_box_style)
        .width(320),
    )
    .style(crate::theme::dialog_scrim_style)
    .into()
}

// ──────────────────────── Helpers ────────────────────────

fn format_timestamp(ts: f64) -> String {
    // Format ms-since-epoch to a human-readable string.
    // Using JS Date for proper locale formatting in WASM.
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ts));
    let year = date.get_full_year();
    let month = date.get_month() + 1; // 0-indexed
    let day = date.get_date();
    let hour = date.get_hours();
    let min = date.get_minutes();
    format!("{year}-{month:02}-{day:02} {hour:02}:{min:02}")
}

/// Opaque tooltip style: dark background with subtle border, avoids visual
/// blending with underlying elements.
pub(crate) fn tooltip_style(_theme: &iced::Theme) -> container::Style {
    crate::theme::tooltip_style(_theme)
}



/// History panel enum (which main panel to show).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HistoryPanel {
    Records,
    Statistics,
}

/// Page enum for multi-page navigation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Page {
    Analysis,
    History {
        panel: HistoryPanel,
        sidebar_open: bool,
    },
}

impl Default for Page {
    fn default() -> Self {
        Self::Analysis
    }
}
