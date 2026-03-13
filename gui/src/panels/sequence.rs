use egui::{Color32, Pos2, Rect, Stroke, Ui, Vec2};

use crate::state::{MessageEntry, MessageKind, ReceiverOutcome};

// -- Palette (no raw hex) --------------------------------------------------
const COLOR_TX_OK: Color32 = Color32::from_rgb(100, 200, 100);
const COLOR_RX: Color32 = Color32::from_rgb(100, 150, 255);
const COLOR_DROP: Color32 = Color32::from_rgb(255, 100, 100);
const COLOR_BIT_ERR: Color32 = Color32::from_rgb(255, 210, 60);
const COLOR_LIFELINE: Color32 = Color32::from_gray(60);
const COLOR_HEADER: Color32 = Color32::from_gray(220);
const COLOR_TS_LABEL: Color32 = Color32::from_gray(100);
const COLOR_HIGHLIGHT: Color32 = Color32::from_rgba_premultiplied(255, 255, 100, 15);

/// Action from the sequence diagram panel.
pub enum SequenceAction {
    None,
    /// User clicked a message arrow; jump to this event index and select the
    /// source node so the Inspector expands it.
    JumpToEvent {
        record_index: usize,
        node: String,
    },
}

/// Show the message sequence diagram.
///
/// Vertical axis = timestep (compacted: only rows with messages are shown).
/// Horizontal axis = one lifeline per node.
///
/// - **TX** events draw solid arrows from sender to each receiver:
///   green = clean receive, yellow = bit errors, red = dropped (with X).
/// - **RX** events draw a short dashed vertical segment on the receiver's
///   lifeline so it is easy to spot who received.
/// - **Drop** events from the message list are rendered as an X on that node's
///   lifeline.
///
/// `zoom` scales both the lifeline spacing and row height. Ctrl+scroll
/// adjusts it in-place.
pub fn show_sequence_diagram(
    ui: &mut Ui,
    messages: &[MessageEntry],
    node_names: &[String],
    current_timestep: u64,
    current_event: Option<usize>,
    zoom: &mut f32,
) -> SequenceAction {
    let mut action = SequenceAction::None;

    if node_names.is_empty() {
        ui.label("No nodes");
        return action;
    }

    // --- Zoom via Ctrl+scroll ------------------------------------------------
    let scroll_delta = ui.input(|i| {
        if i.modifiers.ctrl {
            i.raw_scroll_delta.y
        } else {
            0.0
        }
    });
    if scroll_delta != 0.0 {
        let factor = 1.0 + scroll_delta * 0.002;
        *zoom = (*zoom * factor).clamp(0.15, 5.0);
    }

    let base_row_height = 24.0_f32;
    let base_lifeline_spacing = 100.0_f32;
    let row_height = base_row_height * *zoom;
    let lifeline_spacing = base_lifeline_spacing * *zoom;
    let header_height = 30.0_f32;
    let ts_label_margin = 50.0_f32;

    // Sorted node names for stable ordering
    let mut sorted_names: Vec<String> = node_names.to_vec();
    sorted_names.sort();

    let margin = lifeline_spacing / 2.0;
    let total_width = (sorted_names.len() as f32) * lifeline_spacing;

    // -- Compact row mapping: only timesteps with messages --------------------
    let mut active_timesteps: Vec<u64> = messages.iter().map(|m| m.timestep).collect();
    active_timesteps.sort_unstable();
    active_timesteps.dedup();

    let ts_to_row: std::collections::HashMap<u64, usize> = active_timesteps
        .iter()
        .enumerate()
        .map(|(i, &ts)| (ts, i))
        .collect();

    let current_row = ts_to_row.get(&current_timestep).copied();
    let num_rows = active_timesteps.len();

    // -- Layout ---------------------------------------------------------------
    egui::ScrollArea::both()
        .id_salt("sequence_scroll")
        // When Ctrl is held, suppress the default scroll so it feeds into zoom.
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .show(ui, |ui| {
            // Consume scroll events when Ctrl is held so the scroll area
            // does not move while we are zooming.
            if ui.input(|i| i.modifiers.ctrl) {
                ui.input_mut(|i| {
                    i.smooth_scroll_delta = Vec2::ZERO;
                });
            }

            let total_height = header_height + (num_rows as f32) * row_height + 20.0;

            let (rect, _response) = ui.allocate_exact_size(
                Vec2::new(
                    (total_width + ts_label_margin).max(ui.available_width()),
                    total_height,
                ),
                egui::Sense::click(),
            );

            let painter = ui.painter_at(rect);

            // Lifeline X positions
            let lifeline_x: Vec<f32> = sorted_names
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    rect.left() + ts_label_margin + margin + (i as f32) * lifeline_spacing
                })
                .collect();

            // --- Header (node names) -----------------------------------------
            let font_size = (12.0 * *zoom).clamp(7.0, 18.0);
            for (i, name) in sorted_names.iter().enumerate() {
                let x = lifeline_x[i];
                painter.text(
                    Pos2::new(x, rect.top() + header_height / 2.0),
                    egui::Align2::CENTER_CENTER,
                    name,
                    egui::FontId::proportional(font_size),
                    COLOR_HEADER,
                );
            }

            // --- Lifelines (dashed vertical lines) ---------------------------
            let body_top = rect.top() + header_height;
            let body_bottom = rect.top() + total_height;
            let dash_len = 6.0;
            let gap_len = 4.0;
            for &x in &lifeline_x {
                let mut y = body_top;
                while y < body_bottom {
                    let y_end = (y + dash_len).min(body_bottom);
                    painter.line_segment(
                        [Pos2::new(x, y), Pos2::new(x, y_end)],
                        Stroke::new(1.0, COLOR_LIFELINE),
                    );
                    y += dash_len + gap_len;
                }
            }

            // --- Current timestep highlight band -----------------------------
            if let Some(row) = current_row {
                let y = body_top + (row as f32) * row_height;
                let band = Rect::from_min_max(
                    Pos2::new(rect.left(), y),
                    Pos2::new(rect.right(), y + row_height),
                );
                painter.rect_filled(band, 0.0, COLOR_HIGHLIGHT);
            }

            // --- Timestep labels ---------------------------------------------
            let ts_font = (9.0 * *zoom).clamp(6.0, 14.0);
            for (row, &ts) in active_timesteps.iter().enumerate() {
                let y = body_top + (row as f32) * row_height + row_height / 2.0;
                painter.text(
                    Pos2::new(rect.left() + 2.0, y),
                    egui::Align2::LEFT_CENTER,
                    format!("t={ts}"),
                    egui::FontId::proportional(ts_font),
                    COLOR_TS_LABEL,
                );
            }

            // --- Helper: Y centre for a given timestep -----------------------
            let y_for_ts = |ts: u64| -> Option<f32> {
                ts_to_row
                    .get(&ts)
                    .map(|&row| body_top + (row as f32) * row_height + row_height / 2.0)
            };

            // --- Draw messages -----------------------------------------------
            for msg in messages {
                let Some(y) = y_for_ts(msg.timestep) else {
                    continue;
                };
                let is_current =
                    current_event.is_some() && msg.record_index == current_event;

                match &msg.kind {
                    // ========================================================
                    // TX: draw arrows from sender to every receiver
                    // ========================================================
                    MessageKind::Sent => {
                        let Some(src_x_idx) =
                            sorted_names.iter().position(|n| n == &msg.src_node)
                        else {
                            continue;
                        };
                        let src_x = lifeline_x[src_x_idx];

                        if msg.receivers.is_empty() {
                            // No correlated receivers yet: just a dot on sender
                            let radius = if is_current { 4.0 } else { 2.5 };
                            painter.circle_filled(
                                Pos2::new(src_x, y),
                                radius,
                                COLOR_TX_OK,
                            );
                        } else {
                            // Draw a small dot on the sender lifeline
                            painter.circle_filled(
                                Pos2::new(src_x, y),
                                if is_current { 3.5 } else { 2.0 },
                                COLOR_TX_OK,
                            );

                            for recv in &msg.receivers {
                                let Some(dst_x_idx) =
                                    sorted_names.iter().position(|n| n == &recv.node)
                                else {
                                    continue;
                                };
                                let dst_x = lifeline_x[dst_x_idx];

                                let (recv_color, is_drop) = match &recv.outcome {
                                    ReceiverOutcome::Received if recv.has_bit_errors => {
                                        (COLOR_BIT_ERR, false)
                                    }
                                    ReceiverOutcome::Received => (COLOR_TX_OK, false),
                                    ReceiverOutcome::Dropped(_) => (COLOR_DROP, true),
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
                                if is_drop {
                                    let half = 4.0;
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

                                // Tooltip on hover (drop reason or bit-error note)
                                let hover_rect = Rect::from_center_size(
                                    Pos2::new(dst_x, y),
                                    Vec2::splat(12.0),
                                );
                                if ui.rect_contains_pointer(hover_rect) {
                                    let tip_text = match &recv.outcome {
                                        ReceiverOutcome::Dropped(reason) => {
                                            format!("{} dropped: {reason}", recv.node)
                                        }
                                        ReceiverOutcome::Received
                                            if recv.has_bit_errors =>
                                        {
                                            format!("{}: bit errors", recv.node)
                                        }
                                        ReceiverOutcome::Received => {
                                            format!("{}: received", recv.node)
                                        }
                                    };
                                    egui::containers::popup::show_tooltip_at_pointer(
                                        ui.ctx(),
                                        egui::LayerId::new(
                                            egui::Order::Tooltip,
                                            ui.id().with("recv_tip"),
                                        ),
                                        ui.id().with((
                                            "recv_tip",
                                            msg.timestep,
                                            &recv.node,
                                        )),
                                        |ui| {
                                            ui.label(tip_text);
                                        },
                                    );
                                }
                            }
                        }
                    }

                    // ========================================================
                    // RX: dashed vertical segment on the receiver lifeline
                    // ========================================================
                    MessageKind::Received => {
                        let Some(rx_idx) =
                            sorted_names.iter().position(|n| n == &msg.src_node)
                        else {
                            continue;
                        };
                        let x = lifeline_x[rx_idx];
                        let seg_half = (row_height * 0.35).max(4.0);
                        let dash = 3.0_f32;
                        let gap = 2.0_f32;
                        let thickness = if is_current { 2.5 } else { 1.5 };

                        // Draw dashed vertical segment centred on the row
                        let mut cy = y - seg_half;
                        let y_end = y + seg_half;
                        while cy < y_end {
                            let d_end = (cy + dash).min(y_end);
                            painter.line_segment(
                                [Pos2::new(x, cy), Pos2::new(x, d_end)],
                                Stroke::new(thickness, COLOR_RX),
                            );
                            cy += dash + gap;
                        }

                        // Small filled circle at centre
                        let radius = if is_current { 3.5 } else { 2.0 };
                        painter.circle_filled(Pos2::new(x, y), radius, COLOR_RX);
                    }

                    // ========================================================
                    // Drop (standalone): X on the dropping node's lifeline
                    // ========================================================
                    MessageKind::Dropped(reason) => {
                        let Some(idx) =
                            sorted_names.iter().position(|n| n == &msg.src_node)
                        else {
                            continue;
                        };
                        let x = lifeline_x[idx];
                        let half = 4.0;
                        let stroke =
                            Stroke::new(if is_current { 2.5 } else { 1.5 }, COLOR_DROP);
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

                        // Tooltip
                        let hover_rect =
                            Rect::from_center_size(Pos2::new(x, y), Vec2::splat(12.0));
                        if ui.rect_contains_pointer(hover_rect) {
                            egui::containers::popup::show_tooltip_at_pointer(
                                ui.ctx(),
                                egui::LayerId::new(
                                    egui::Order::Tooltip,
                                    ui.id().with("drop_tip"),
                                ),
                                ui.id().with(("drop_tip", msg.timestep, &msg.src_node)),
                                |ui| {
                                    ui.label(format!("Dropped: {reason}"));
                                },
                            );
                        }
                    }
                }

                // -- Click detection (whole row) ------------------------------
                if let Some(record_idx) = msg.record_index {
                    let src_idx =
                        sorted_names.iter().position(|n| n == &msg.src_node);
                    if let Some(idx) = src_idx {
                        let x = lifeline_x[idx];
                        let hit_rect = Rect::from_center_size(
                            Pos2::new(x, y),
                            Vec2::new(lifeline_spacing, row_height),
                        );
                        if ui.rect_contains_pointer(hit_rect)
                            && ui.input(|i| i.pointer.any_click())
                        {
                            action = SequenceAction::JumpToEvent {
                                record_index: record_idx,
                                node: msg.src_node.clone(),
                            };
                        }
                    }
                }
            }
        });

    action
}
