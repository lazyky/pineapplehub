//! Diablo 2 Resurrected-style help overlay.
//!
//! **Lines & dots** are drawn on a Canvas; **labels** are iced `text`
//! widgets positioned via standard layout combinators, because Canvas
//! `fill_text` has rendering issues under iced's WebGL backend with
//! multiple text calls per frame.
//! Photo sketch: cropped 611×346

use crate::theme;
use crate::Message;
use iced::widget::canvas::{self, Geometry, Path, Stroke, Text as CanvasText};
use iced::widget::{button, canvas as canvas_widget, column, container, image, row, space, stack, text};
use iced::{Color, Element, Length, Padding, Point, Rectangle, Size, Theme, mouse};

// ────────────────────────  Annotation  ────────────────────────

struct Annotation {
    /// Pixel x of the target.
    tx: f32,
    /// Pixel y of the target.
    ty: f32,
    /// Proportional y of the label text (0–1 of viewport height).
    label_frac_y: f32,
}

// ────────────────────────  Layout constants  ────────────────────────

const TB_PAD_Y: f32 = 14.0;
const TB_PAD_X: f32 = 20.0;
const TB_SPACING: f32 = 8.0;

const ICON_SZ: f32 = 20.0;
const ICON_BTN_PAD: f32 = 6.0;
const ICON_BTN_W: f32 = ICON_SZ + ICON_BTN_PAD * 2.0;

const LOGO_H: f32 = 34.0;
const LOGO_PAD_X: f32 = 8.0;
const LOGO_FAVICON_SZ: f32 = 26.0;

const TB_H: f32 = TB_PAD_Y + LOGO_H + TB_PAD_Y;
const TB_CENTER_Y: f32 = TB_H / 2.0;




// ────────────────────────  Canvas (lines & dots only)  ────────────

struct LinesCanvas {
    build: fn(f32, f32) -> Vec<Annotation>,
}

impl canvas::Program<Message> for LinesCanvas {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let w = bounds.width;
        let h = bounds.height;

        // ── Scrim ──
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(w, h),
            Color { a: 0.82, ..theme::BG_DEEP },
        );

        let line_color = Color::from_rgba(1.0, 0.85, 0.3, 0.50);
        let dot_r = 4.0_f32;

        let annotations = (self.build)(w, h);

        for ann in &annotations {
            let px = ann.tx;
            let target_py = ann.ty;
            let label_py = ann.label_frac_y * h;

            // Vertical line from target toward label
            let (la, lb) = if label_py > target_py {
                (target_py, label_py + 4.0)
            } else {
                (target_py, label_py + 18.0)
            };
            if (lb - la).abs() > 4.0 {
                frame.stroke(
                    &Path::line(Point::new(px, la), Point::new(px, lb)),
                    Stroke::default().with_color(line_color).with_width(1.5),
                );
            }

            // Target dot
            frame.fill(&Path::circle(Point::new(px, target_py), dot_r), theme::ACCENT);
        }

        // Close hint — only drawn when there are annotations
        if !annotations.is_empty() {
            let hint = "Press F1 / ESC to close     ·     Click anywhere to dismiss";
            frame.fill_text(CanvasText {
                content: hint.to_string(),
                position: Point::new(w * 0.17, h - 30.0),
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.45),
                size: iced::Pixels(13.0),
                ..CanvasText::default()
            });
        }

        vec![frame.into_geometry()]
    }
}

// ────────────────────────  Annotation data  ────────────────────────

fn analysis_annotations(w: f32, _h: f32) -> Vec<Annotation> {
    let github_cx = w - TB_PAD_X - ICON_BTN_W / 2.0;
    let history_cx = github_cx - TB_SPACING - ICON_BTN_W;
    let logo_cx = TB_PAD_X + LOGO_PAD_X + LOGO_FAVICON_SZ / 2.0;

    vec![
        Annotation { tx: logo_cx, ty: TB_CENTER_Y, label_frac_y: 0.20 },
        Annotation { tx: history_cx, ty: TB_CENTER_Y, label_frac_y: 0.30 },
        Annotation { tx: github_cx, ty: TB_CENTER_Y, label_frac_y: 0.42 },
    ]
}

fn history_annotations(_w: f32, _h: f32) -> Vec<Annotation> {
    // History page uses text-panel help, no annotations needed.
    // This function is only kept for the LinesCanvas interface; it draws only scrim.
    vec![]
}

// ────────────────────────  Styled label  ────────────────────────

fn lbl<'a>(s: &str) -> Element<'a, Message> {
    text(s.to_string())
        .size(15)
        .color(theme::ACCENT)
        .into()
}

// ────────────────────────  Public API  ────────────────────────

pub(crate) fn view_analysis_help<'a>() -> Element<'a, Message> {
    let lines = canvas_widget(LinesCanvas { build: analysis_annotations })
        .width(Length::Fill)
        .height(Length::Fill);

    // Labels use FillPortion spacers to match Canvas label_frac_y exactly:
    // Logo=0.20, History=0.30, GitHub=0.42
    let labels: Element<'_, Message> = column![
        // 0 → 0.20
        space::vertical().height(Length::FillPortion(20)),
        container(lbl("Logo — Back to Analysis"))
            .padding(Padding::from([0, 0]).left(10)),
        // 0.20 → 0.30
        space::vertical().height(Length::FillPortion(10)),
        // History (right-aligned)
        container(
            row![
                space::horizontal().width(Length::Fill),
                lbl("History"),
                space::horizontal().width(Length::Fixed(85.0)),
            ]
        ).width(Length::Fill),
        // 0.30 → 0.42
        space::vertical().height(Length::FillPortion(12)),
        // GitHub (right-aligned)
        container(
            row![
                space::horizontal().width(Length::Fill),
                lbl("GitHub Repository"),
                space::horizontal().width(Length::Fixed(15.0)),
            ]
        ).width(Length::Fill),
        // 0.42 → 1.0 — Photo example sketch filling the remaining space
        space::vertical().height(Length::FillPortion(1)),
        container(
            column![
                container(
                    image(image::Handle::from_bytes(
                        include_bytes!("../../assets/help_sketch.png").as_slice(),
                    ))
                    .width(Length::Fill)
                    .height(Length::Fill)
                )
                .center_x(Length::Fill)
                .width(Length::Fill)
                .height(Length::Fill),
                container(
                    text("Photo Example")
                        .size(13)
                        .color(Color::from_rgba(1.0, 1.0, 1.0, 0.55))
                )
                .center_x(Length::Fill),
            ]
        )
        .width(Length::Fill)
        .height(Length::FillPortion(48)),
        space::vertical().height(Length::FillPortion(10)),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into();

    let overlay = stack![lines, labels]
        .width(Length::Fill)
        .height(Length::Fill);

    button(overlay)
        .on_press(Message::ToggleHelp)
        .style(theme::text_button_style)
        .padding(0)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// A single styled paragraph line for the history help panel.
fn help_line<'a>(s: &str) -> Element<'a, Message> {
    text(s.to_string())
        .size(14)
        .color(Color::from_rgba(0.95, 0.92, 0.85, 0.92))
        .into()
}

/// A section heading for the history help panel.
fn help_heading<'a>(s: &str) -> Element<'a, Message> {
    text(s.to_string())
        .size(16)
        .color(theme::ACCENT)
        .into()
}

pub(crate) fn view_history_help<'a>() -> Element<'a, Message> {
    // Scrim (Canvas draws only the dark overlay, no annotations)
    let scrim = canvas_widget(LinesCanvas { build: history_annotations })
        .width(Length::Fill)
        .height(Length::Fill);

    // Centered help text panel
    let panel = container(
        column![
            text("History — Quick Guide")
                .size(20)
                .color(theme::ACCENT),
            space::vertical().height(Length::Fixed(16.0)),

            help_heading("Star"),
            help_line("Click the star icon next to a session to protect it."),
            help_line("Starred sessions will never be auto-cleaned."),
            space::vertical().height(Length::Fixed(10.0)),

            help_heading("Data Storage & Cleanup"),
            help_line("All analysis results are saved in your browser (IndexedDB)."),
            help_line("Up to 5 000 records can be stored — no server needed."),
            help_line("When storage gets full, use \"Quick Cleanup\" at the bottom"),
            help_line("left to remove the oldest unstarred sessions automatically."),
            help_line("Starred sessions are always kept safe."),
            space::vertical().height(Length::Fixed(10.0)),

            help_heading("Records Tab"),
            help_line("View every individual measurement from selected sessions."),
            help_line("Orange rows are auto-flagged statistical outliers (IQR method:"),
            help_line("values outside Q1\u{2013}1.5\u{00d7}IQR ~ Q3+1.5\u{00d7}IQR)."),
            help_line("You can also manually flag a row as suspect, add free-text"),
            help_line("notes, or edit the height / width values if needed."),
            space::vertical().height(Length::Fixed(10.0)),

            help_heading("Statistics Tab"),
            help_line("Summary statistics (mean, median, SD, min, max, Q1, Q3)"),
            help_line("computed across all selected sessions at a glance."),
            space::vertical().height(Length::Fixed(16.0)),

            text("Press F1 / ESC to close   ·   Click anywhere to dismiss")
                .size(13)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.40)),
        ]
        .spacing(3)
        .align_x(iced::Alignment::Center)
    )
    .width(Length::Shrink)
    .max_width(560)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .padding(40);

    let overlay = stack![scrim, panel]
        .width(Length::Fill)
        .height(Length::Fill);

    button(overlay)
        .on_press(Message::ToggleHelp)
        .style(theme::text_button_style)
        .padding(0)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
