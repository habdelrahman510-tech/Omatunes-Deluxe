//! Equalizer panel for the right-hand pane.

use iced::widget::canvas::{self, Canvas, Frame, Stroke};
use iced::widget::{button, column, container, pick_list, row, scrollable, slider, text, Space};
use iced::{Alignment, Element, Length, Point, Rectangle, Size};

use crate::app::{AppState, Message};
use crate::audio::eq::{
    self, EqSettings, BAND_COUNT, BAND_FREQS, GAIN_MAX, GAIN_MIN, PREAMP_MAX, PREAMP_MIN,
};
use crate::ui::theme;

/// Vertical half-range of the response curve, in dB.
const CURVE_DB_RANGE: f32 = 15.0;

pub fn view(state: &AppState) -> Element<'_, Message> {
    let settings = state.equalizer;

    let header = row![
        power_button(settings.enabled),
        preset_picker(&settings),
        // Keep clear of the panel's floating close button.
        Space::with_width(26),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let curve = Canvas::new(EqCurve { settings })
        .width(Length::Fill)
        .height(Length::Fixed(104.0));

    let mut bands = column![].spacing(3).width(Length::Fill);
    for band in 0..BAND_COUNT {
        let gain = settings.bands_db[band];
        bands = bands.push(
            row![
                text(band_label(BAND_FREQS[band]))
                    .size(11)
                    .color(theme::subtext())
                    .width(Length::Fixed(44.0)),
                slider(GAIN_MIN..=GAIN_MAX, gain, move |value| {
                    Message::EqBandChanged(band, value)
                })
                .step(0.5_f32)
                .width(Length::Fill),
                text(format!("{gain:+.1}"))
                    .size(11)
                    .color(theme::text())
                    .width(Length::Fixed(40.0))
                    .align_x(iced::alignment::Horizontal::Right),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
    }

    let preamp = row![
        text("Pre")
            .size(11)
            .color(theme::subtext())
            .width(Length::Fixed(44.0)),
        slider(PREAMP_MIN..=PREAMP_MAX, settings.preamp_db, Message::EqPreampChanged)
            .step(0.5_f32)
            .width(Length::Fill),
        text(format!("{:+.1}", settings.effective_preamp()))
            .size(11)
            .color(theme::text())
            .width(Length::Fixed(40.0))
            .align_x(iced::alignment::Horizontal::Right),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let footer = row![
        text("Auto headroom · 10 bands")
            .size(10)
            .color(theme::overlay0())
            .width(Length::Fill),
        button(text("Reset").size(11))
            .on_press(Message::EqReset)
            .padding([4, 10])
            .style(theme::secondary_button),
    ]
    .align_y(Alignment::Center);

    let body = column![
        header,
        Space::with_height(8),
        curve,
        Space::with_height(8),
        scrollable(
            column![
                bands,
                Space::with_height(6),
                preamp,
                Space::with_height(8),
                footer,
            ]
            .width(Length::Fill)
        )
        .height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding([10, 12])
        .into()
}

fn power_button(enabled: bool) -> Element<'static, Message> {
    button(text(if enabled { "On" } else { "Off" }).size(12))
        .on_press(Message::EqToggled(!enabled))
        .padding([5, 14])
        .style(move |_, status| {
            use iced::widget::button::Status;

            let hovered = matches!(status, Status::Hovered | Status::Pressed);
            let background = if enabled {
                theme::accent()
            } else if hovered {
                theme::overlay0()
            } else {
                theme::surface0()
            };

            iced::widget::button::Style {
                background: Some(iced::Background::Color(background)),
                text_color: if enabled { theme::base() } else { theme::subtext() },
                border: iced::Border {
                    radius: 4.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .into()
}

fn preset_picker(settings: &EqSettings) -> Element<'static, Message> {
    let names: Vec<&'static str> = eq::BUILTIN_PRESETS.iter().map(|p| p.name).collect();
    let selected = eq::preset_name(settings);

    pick_list(names, Some(selected), |name: &'static str| {
        Message::EqPresetSelected(name.to_string())
    })
    .text_size(12)
    .padding(6)
    .width(Length::Fill)
    .into()
}

fn band_label(freq: f32) -> String {
    if freq >= 1000.0 {
        format!("{:.0}k", freq / 1000.0)
    } else if freq.fract().abs() > 0.01 {
        format!("{freq:.1}")
    } else {
        format!("{freq:.0}")
    }
}

// ── Response curve ────────────────────────────────────────────────────────────

struct EqCurve {
    settings: EqSettings,
}

impl<Message> canvas::Program<Message> for EqCurve {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());

        let width = bounds.width;
        let height = bounds.height;
        let pad = 3.0;
        let span = width - 2.0 * pad;
        let vertical = height - 2.0 * pad;

        let y_of = |db: f32| {
            pad + (CURVE_DB_RANGE - db.clamp(-CURVE_DB_RANGE, CURVE_DB_RANGE))
                / (2.0 * CURVE_DB_RANGE)
                * vertical
        };
        let x_of = |t: f32| pad + t * span;

        for db in [-12.0f32, -6.0, 0.0, 6.0, 12.0] {
            let alpha = if db == 0.0 { 0.65 } else { 0.26 };
            frame.fill_rectangle(
                Point::new(pad, y_of(db)),
                Size::new(span, 1.0),
                theme::with_alpha(theme::overlay0(), alpha),
            );
        }

        let decade = 20000f32.log10() - 20f32.log10();
        for freq in [100.0f32, 1000.0, 10000.0] {
            let t = (freq.log10() - 20f32.log10()) / decade;
            frame.fill_rectangle(
                Point::new(x_of(t), pad),
                Size::new(1.0, vertical),
                theme::with_alpha(theme::overlay0(), 0.22),
            );
        }

        let steps = 160usize;
        let mut points = Vec::with_capacity(steps + 1);
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            let freq = 20.0 * 1000f32.powf(t);
            let db = eq::response_db(&self.settings, freq, 48_000.0);
            points.push(Point::new(x_of(t), y_of(db)));
        }

        let color = if self.settings.enabled {
            theme::accent()
        } else {
            theme::subtext()
        };

        // Shaded area under the curve, for a bit of body.
        let baseline = y_of(-CURVE_DB_RANGE);
        let mut area = canvas::path::Builder::new();
        area.move_to(Point::new(points[0].x, baseline));
        for point in &points {
            area.line_to(*point);
        }
        area.line_to(Point::new(points[points.len() - 1].x, baseline));
        area.close();
        frame.fill(&area.build(), theme::with_alpha(color, 0.12));

        let mut line = canvas::path::Builder::new();
        line.move_to(points[0]);
        for point in &points[1..] {
            line.line_to(*point);
        }
        frame.stroke(
            &line.build(),
            Stroke::default().with_color(color).with_width(1.8),
        );

        vec![frame.into_geometry()]
    }
}
