use egui::{Color32, Pos2, Rect, Stroke, Ui, Vec2};

use crate::state::{MessageEntry, MessageKind};

/// Action from the sequence diagram panel.
pub enum SequenceAction {
    None,
    /// User clicked a message arrow; jump to this event.
    JumpToEvent(usize),
}

/// Show the message sequence diagram.
///
/// Vertical axis = timestep, horizontal axis = one lifeline per node.
/// TX arrows drawn from sender to each receiver. Drops shown as arrows ending in X.
pub fn show_sequence_diagram(
    ui: &mut Ui,
    messages: &[MessageEntry],
    node_names: &[String],
    current_timestep: u64,
    current_event: Option<usize>,
) -> SequenceAction {
    let mut action = SequenceAction::None;

    if node_names.is_empty() {
        ui.label("No nodes");
        return action;
    }

    let header_height = 30.0;
    let row_height = 20.0;
    let lifeline_spacing = 100.0_f32;

    // Compute the total width needed
    let total_width = (node_names.len() as f32) * lifeline_spacing;

    // Sorted node names for stable ordering
    let mut sorted_names: Vec<String> = node_names.to_vec();
    sorted_names.sort();

    // Calculate lifeline X positions
    let margin = lifeline_spacing / 2.0;

    egui::ScrollArea::both()
        .id_salt("sequence_scroll")
        .show(ui, |ui| {
            // Determine how many timesteps we need to show
            let max_ts = messages.iter().map(|m| m.timestep).max().unwrap_or(0);
            let total_height = header_height + ((max_ts + 1) as f32) * row_height + 20.0;

            let (rect, _response) = ui.allocate_exact_size(
                Vec2::new(total_width.max(ui.available_width()), total_height),
                egui::Sense::click(),
            );

            let painter = ui.painter_at(rect);

            let lifeline_x: Vec<f32> = sorted_names
                .iter()
                .enumerate()
                .map(|(i, _)| rect.left() + margin + (i as f32) * lifeline_spacing)
                .collect();

            // Draw header (node names)
            for (i, name) in sorted_names.iter().enumerate() {
                let x = lifeline_x[i];
                painter.text(
                    Pos2::new(x, rect.top() + header_height / 2.0),
                    egui::Align2::CENTER_CENTER,
                    name,
                    egui::FontId::proportional(12.0),
                    Color32::from_gray(220),
                );
            }

            // Draw lifelines (vertical dashed lines)
            let body_top = rect.top() + header_height;
            let body_bottom = rect.top() + total_height;
            for &x in &lifeline_x {
                // Draw as a series of short segments (dashed effect)
                let dash_len = 6.0;
                let gap_len = 4.0;
                let mut y = body_top;
                while y < body_bottom {
                    let y_end = (y + dash_len).min(body_bottom);
                    painter.line_segment(
                        [Pos2::new(x, y), Pos2::new(x, y_end)],
                        Stroke::new(1.0, Color32::from_gray(60)),
                    );
                    y += dash_len + gap_len;
                }
            }

            // Draw current timestep highlight band
            {
                let y = body_top + (current_timestep as f32) * row_height;
                let band = Rect::from_min_max(
                    Pos2::new(rect.left(), y),
                    Pos2::new(rect.right(), y + row_height),
                );
                painter.rect_filled(
                    band,
                    0.0,
                    Color32::from_rgba_premultiplied(255, 255, 100, 15),
                );
            }

            // Draw timestep labels on the left
            let label_step = if max_ts > 200 { 50 } else if max_ts > 50 { 10 } else { 5 };
            for ts in (0..=max_ts).step_by(label_step.max(1) as usize) {
                let y = body_top + (ts as f32) * row_height + row_height / 2.0;
                painter.text(
                    Pos2::new(rect.left() + 2.0, y),
                    egui::Align2::LEFT_CENTER,
                    format!("{ts}"),
                    egui::FontId::proportional(9.0),
                    Color32::from_gray(100),
                );
            }

            // Draw message arrows
            for msg in messages {
                let src_idx = sorted_names.iter().position(|n| n == &msg.src_node);

                let (color, is_current) = match &msg.kind {
                    MessageKind::Sent => {
                        let c = Color32::from_rgb(100, 200, 100);
                        let is_cur = current_event.is_some() && msg.record_index == current_event;
                        (c, is_cur)
                    }
                    MessageKind::Received => {
                        // RX: draw as a small dot on the receiving lifeline
                        let c = Color32::from_rgb(100, 150, 255);
                        let is_cur = current_event.is_some() && msg.record_index == current_event;
                        if let Some(idx) = src_idx {
                            let x = lifeline_x[idx];
                            let y =
                                body_top + (msg.timestep as f32) * row_height + row_height / 2.0;
                            let radius = if is_cur { 4.0 } else { 2.5 };
                            painter.circle_filled(Pos2::new(x, y), radius, c);
                        }
                        (c, is_cur)
                    }
                    MessageKind::Dropped(reason) => {
                        let c = Color32::from_rgb(255, 100, 100);
                        let is_cur = current_event.is_some() && msg.record_index == current_event;
                        // Draw X on the source lifeline
                        if let Some(idx) = src_idx {
                            let x = lifeline_x[idx];
                            let y =
                                body_top + (msg.timestep as f32) * row_height + row_height / 2.0;
                            let half = 3.5;
                            let stroke = Stroke::new(
                                if is_cur { 2.5 } else { 1.5 },
                                c,
                            );
                            painter.line_segment(
                                [
                                    Pos2::new(x - half, y - half),
                                    Pos2::new(x + half, y + half),
                                ],
                                stroke,
                            );
                            painter.line_segment(
                                [
                                    Pos2::new(x - half, y + half),
                                    Pos2::new(x + half, y - half),
                                ],
                                stroke,
                            );
                            // Show reason on hover
                            let hover_rect = Rect::from_center_size(
                                Pos2::new(x, y),
                                Vec2::splat(10.0),
                            );
                            if ui.rect_contains_pointer(hover_rect) {
                                egui::containers::popup::show_tooltip_at_pointer(
                                    ui.ctx(),
                                    egui::LayerId::new(
                                        egui::Order::Tooltip,
                                        ui.id().with("drop_tip"),
                                    ),
                                    ui.id().with(("drop_tip", msg.timestep)),
                                    |ui| {
                                        ui.label(format!("Dropped: {reason}"));
                                    },
                                );
                            }
                        }
                        (c, is_cur)
                    }
                };

                // For TX messages with receivers, draw arrows to each receiver
                if msg.kind == MessageKind::Sent && !msg.receivers.is_empty() {
                    if let Some(src_x_idx) = src_idx {
                        let src_x = lifeline_x[src_x_idx];
                        let y = body_top + (msg.timestep as f32) * row_height + row_height / 2.0;

                        for recv in &msg.receivers {
                            if let Some(dst_x_idx) =
                                sorted_names.iter().position(|n| n == &recv.node)
                            {
                                let dst_x = lifeline_x[dst_x_idx];
                                let recv_color = match &recv.outcome {
                                    crate::state::ReceiverOutcome::Received => {
                                        Color32::from_rgb(100, 200, 100)
                                    }
                                    crate::state::ReceiverOutcome::Dropped(_) => {
                                        Color32::from_rgb(255, 100, 100)
                                    }
                                };
                                let thickness = if is_current { 2.5 } else { 1.5 };

                                // Arrow line
                                painter.line_segment(
                                    [Pos2::new(src_x, y), Pos2::new(dst_x, y)],
                                    Stroke::new(thickness, recv_color),
                                );

                                // Arrowhead
                                let dir = if dst_x > src_x { 1.0_f32 } else { -1.0 };
                                let tip = Pos2::new(dst_x, y);
                                let arrow_len = 6.0;
                                let arrow_width = 3.0;
                                let base_x = tip.x - dir * arrow_len;
                                painter.add(egui::Shape::convex_polygon(
                                    vec![
                                        tip,
                                        Pos2::new(base_x, y - arrow_width),
                                        Pos2::new(base_x, y + arrow_width),
                                    ],
                                    recv_color,
                                    Stroke::NONE,
                                ));

                                // Drop X at destination
                                if matches!(
                                    recv.outcome,
                                    crate::state::ReceiverOutcome::Dropped(_)
                                ) {
                                    let half = 3.5;
                                    let stroke = Stroke::new(2.0, recv_color);
                                    painter.line_segment(
                                        [
                                            Pos2::new(dst_x - half, y - half),
                                            Pos2::new(dst_x + half, y + half),
                                        ],
                                        stroke,
                                    );
                                    painter.line_segment(
                                        [
                                            Pos2::new(dst_x - half, y + half),
                                            Pos2::new(dst_x + half, y - half),
                                        ],
                                        stroke,
                                    );
                                }
                            }
                        }
                    }
                } else if msg.kind == MessageKind::Sent && msg.receivers.is_empty() {
                    // TX without receiver info: just draw a dot on the sender lifeline
                    if let Some(idx) = src_idx {
                        let x = lifeline_x[idx];
                        let y = body_top + (msg.timestep as f32) * row_height + row_height / 2.0;
                        let radius = if is_current { 4.0 } else { 2.5 };
                        painter.circle_filled(Pos2::new(x, y), radius, color);
                    }
                }

                // Click detection for message arrows
                if let Some(record_idx) = msg.record_index {
                    if let Some(idx) = src_idx {
                        let x = lifeline_x[idx];
                        let y = body_top + (msg.timestep as f32) * row_height + row_height / 2.0;
                        let hit_rect =
                            Rect::from_center_size(Pos2::new(x, y), Vec2::new(total_width, row_height));
                        if ui.rect_contains_pointer(hit_rect)
                            && ui.input(|i| i.pointer.any_click())
                        {
                            action = SequenceAction::JumpToEvent(record_idx);
                        }
                    }
                }
            }
        });

    action
}
