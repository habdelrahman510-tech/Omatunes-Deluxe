// Split out of app.rs; methods only, no logic changes.
use super::*;

impl AppState {
    pub(super) fn queue_popover_view(&self) -> Element<'_, Message> {
        use crate::ui::theme;
        use crate::ui::txt::text;
        use iced::widget::{button, column, container, mouse_area, row, scrollable, stack, Space};
        use iced::{Alignment, Length};

        // Dismiss layer: transparent full-window click target behind the panel
        let dismiss_layer = mouse_area(
            container(Space::new(Length::Fill, Length::Fill))
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_| iced::widget::container::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgba(
                        0.0, 0.0, 0.0, 0.0,
                    ))),
                    ..Default::default()
                }),
        )
        .on_press(Message::CloseQueuePopover);

        // Header row: "Queue" label + count + Clear button
        let queue_count = self.queue.len();
        let header = row![
            text(format!("Queue ({queue_count})"))
                .size(12)
                .font(crate::ui::icons::UI_FONT_BOLD)
                .color(theme::subtext())
                .width(Length::Fill),
            button(text("Clear").size(11).color(theme::red()))
                .on_press(Message::ClearQueue)
                .style(
                    move |_: &iced::Theme, status: iced::widget::button::Status| {
                        let hovered = status == iced::widget::button::Status::Hovered
                            || status == iced::widget::button::Status::Pressed;
                        iced::widget::button::Style {
                            text_color: theme::red(),
                            background: if hovered {
                                Some(iced::Background::Color(theme::surface0()))
                            } else {
                                None
                            },
                            border: iced::Border {
                                color: if hovered {
                                    theme::red()
                                } else {
                                    iced::Color::TRANSPARENT
                                },
                                width: 1.0,
                                radius: 4.0.into(),
                            },
                            ..Default::default()
                        }
                    }
                )
                .padding([2, 6]),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .padding([8, 12]);

        // Queue rows
        let current_track_id = self.current_track.as_ref().map(|t| t.id);
        let mut rows: Vec<Element<'_, Message>> = Vec::new();

        if self.queue.is_empty() {
            rows.push(
                container(
                    text("The play queue is empty.")
                        .size(13)
                        .color(theme::overlay0()),
                )
                .padding([12, 12])
                .width(Length::Fill)
                .into(),
            );
        } else {
            for (idx, track) in self.queue.iter().enumerate() {
                let is_current = current_track_id == Some(track.id);
                let title_color = if is_current {
                    theme::accent()
                } else {
                    theme::text()
                };

                // Drag handle
                let drag_handle = mouse_area(
                    container(
                        text("\u{f0c9}")
                            .font(crate::ui::icons::NERD_FONT_MONO)
                            .color(if self.dragging_queue_index == Some(idx) {
                                theme::accent()
                            } else {
                                theme::overlay0()
                            })
                            .size(11),
                    )
                    .padding([4, 6]),
                )
                .on_press(Message::QueueDragStart(idx))
                .on_release(Message::QueueDragEnd)
                .interaction(iced::mouse::Interaction::Grab);

                // Remove (✕) button
                let remove_btn = button(
                    text("\u{f00d}")
                        .font(crate::ui::icons::NERD_FONT_MONO)
                        .size(12)
                        .color(theme::red()),
                )
                .on_press(Message::RemoveQueueTrack(idx))
                .style(iced::widget::button::text)
                .padding([4, 4]);

                // Track info
                let title_txt = text(track.title.clone())
                    .size(13)
                    .color(title_color)
                    .width(Length::Fill);
                let artist_txt = text(track.artist.clone())
                    .size(13)
                    .color(theme::subtext())
                    .width(Length::Fill);

                let info_col = column![title_txt, artist_txt]
                    .spacing(2)
                    .width(Length::Fill);

                // Position number
                let pos_txt = text(format!("{}", idx + 1))
                    .size(13)
                    .color(theme::overlay0())
                    .width(Length::Fixed(20.0));

                let track_row_inner = row![drag_handle, pos_txt, info_col, remove_btn,]
                    .spacing(4)
                    .align_y(Alignment::Center)
                    .padding([4, 8]);

                // Detect if background is light or dark to compute the custom saturated panel style
                let base_color = theme::base();
                let is_dark = (base_color.r + base_color.g + base_color.b) / 3.0 < 0.5;

                // Alternate background logic:
                // We construct two contrasting colors using base and mantle, slightly shifting saturation/lightness.
                let row_bg_even = if is_current {
                    Some(iced::Background::Color(theme::with_alpha(
                        theme::accent(),
                        0.12,
                    )))
                } else if idx % 2 == 1 {
                    if is_dark {
                        // Less saturated/slightly lighter for alternate rows on dark theme
                        Some(iced::Background::Color(theme::mantle()))
                    } else {
                        // More saturated/slightly darker for alternate rows on light theme
                        Some(iced::Background::Color(theme::mantle()))
                    }
                } else {
                    if is_dark {
                        Some(iced::Background::Color(theme::base()))
                    } else {
                        None
                    }
                };

                let mut row_element: Element<'_, Message> = mouse_area(
                    container(track_row_inner)
                        .width(Length::Fill)
                        .style(move |_| iced::widget::container::Style {
                            background: row_bg_even,
                            ..Default::default()
                        }),
                )
                .on_press(Message::PlayQueueTrack(idx))
                .into();

                // Drag-over highlight: when dragging, wrap with mouse_area to detect hover
                if self.dragging_queue_index.is_some() {
                    row_element = mouse_area(row_element)
                        .on_enter(Message::QueueDragOver(idx))
                        .into();
                }

                rows.push(row_element);
            }
        }

        let scroll_content = scrollable(column(rows).spacing(0).width(Length::Fill))
            .id(self.queue_scroll_id.clone())
            .height(Length::Shrink);

        // The panel itself (30% wider: 360 * 1.3 = 468)
        let panel_content = column![
            header,
            container(Space::new(Length::Fill, Length::Fixed(1.0)))
                .style(|_| iced::widget::container::Style {
                    background: Some(iced::Background::Color(theme::surface0())),
                    ..Default::default()
                })
                .width(Length::Fill),
            scroll_content,
        ]
        .spacing(0)
        .width(Length::Fixed(468.0));

        // Background color styling based on theme light/dark saturation shift
        let base_col = theme::base();
        let is_dark = (base_col.r + base_col.g + base_col.b) / 3.0 < 0.5;
        let popover_bg = if is_dark {
            // For dark backgrounds: blend with mantle to make it slightly less saturated / deeper
            theme::lerp_color(base_col, theme::mantle(), 0.5)
        } else {
            // For light backgrounds: blend with mantle or surface0 to make it more saturated / distinct
            theme::lerp_color(base_col, theme::surface0(), 0.15)
        };

        let panel = container(panel_content)
            .width(Length::Fixed(468.0))
            .max_height(706.0) // 20% taller: 588 * 1.2 = 706
            .style(move |_| iced::widget::container::Style {
                background: Some(iced::Background::Color(popover_bg)),
                border: iced::Border {
                    color: theme::surface0(),
                    width: 1.0,
                    radius: 8.0.into(),
                },
                shadow: iced::Shadow {
                    color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.4),
                    offset: [0.0, 4.0].into(),
                    blur_radius: 12.0,
                },
                ..Default::default()
            });

        // Position: Anchored directly above the floating Now Playing button at the bottom
        let panel_left_offset = (self.sidebar_width.round() + 18.0).max(0.0);

        let positioned_panel = container(panel)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced::alignment::Horizontal::Left)
            .align_y(iced::alignment::Vertical::Bottom)
            .padding(iced::Padding {
                top: 0.0,
                left: panel_left_offset,
                right: 0.0,
                bottom: 68.0,
            });

        // Stack: dismiss layer behind, panel in front
        stack![dismiss_layer, positioned_panel,]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub(super) fn shortcuts_modal_view(&self) -> Element<'_, Message> {
        let title = text("Keyboard Shortcuts")
            .size(20)
            .font(crate::ui::icons::UI_FONT_BOLD)
            .color(theme::accent());

        let row_item = |keys: &'static str, desc: &'static str| {
            row![
                text(keys)
                    .width(Length::Fixed(120.0))
                    .font(crate::ui::icons::UI_FONT_BOLD)
                    .color(theme::accent())
                    .size(13),
                text(desc).color(theme::text()).size(13),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
        };

        let content = iced::widget::column![
            row![
                title,
                Space::with_width(Length::Fill),
                button(
                    text("\u{f00d}")
                        .font(crate::ui::icons::NERD_FONT_MONO)
                        .color(theme::red())
                        .size(16)
                )
                .on_press(Message::CloseShortcuts)
                .style(iced::widget::button::text)
            ]
            .align_y(Alignment::Center),
            Space::with_height(16),
            row_item("Space", "Play / Pause / Play Selected Track"),
            row_item("N", "Next Track"),
            row_item("P", "Previous Track"),
            row_item("L / F", "Like / Unlike Song"),
            row_item("E", "Edit Metadata Tags"),
            row_item("C", "Create Custom Playlist"),
            row_item("A", "Add Current Song to Playlist"),
            row_item("Arrow Up/Down", "Navigate Lists (Sidebar/Tracks)"),
            row_item("F5", "Rescan Music Library Folder"),
            row_item("+ / -", "Increase / Decrease Volume"),
            row_item("] / [", "Increase / Decrease Scaling"),
            row_item("Right/Left", "Seek Forward / Backward"),
            row_item("Tab", "Focus next field / cycle ID3 inputs"),
            row_item("Shift + Tab", "Cycle ID3 input backwards"),
            row_item("/", "Focus song search input"),
        ]
        .spacing(10)
        .padding(24);

        let dialog = container(content)
            .width(420)
            .style(|_| iced::widget::container::Style {
                background: Some(iced::Background::Color(theme::base())),
                border: iced::Border {
                    color: theme::accent(),
                    width: 1.0,
                    radius: 8.0.into(),
                },
                shadow: iced::Shadow {
                    color: theme::mantle(),
                    offset: [0.0, 4.0].into(),
                    blur_radius: 12.0,
                },
                ..Default::default()
            });

        container(dialog)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(|_| iced::widget::container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.6,
                ))),
                ..Default::default()
            })
            .into()
    }

    pub(super) fn header_view(&self) -> Element<'_, Message> {
        container(
            row![
                text(crate::ui::icons::ICON_MUSIC)
                    .font(crate::ui::icons::NERD_FONT_MONO)
                    .color(theme::accent())
                    .size(16),
                Space::with_width(6),
                text("omatunes")
                    .color(theme::accent())
                    .size(16)
                    .font(crate::ui::icons::UI_FONT_BOLD),
            ]
            .align_y(Alignment::Center),
        )
        .style(theme::header)
        .width(Length::Fill)
        .padding([0, 16])
        .into()
    }
}
