use egui::{Pos2, Rect, Stroke, Ui, Vec2};

use crate::constants::*;
use crate::state::{MessageEntry, MessageKind, ReceiverOutcome};

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
        let factor = 1.0 + scroll_delta * GRID_SCROLL_ZOOM_FACTOR;
        *zoom = (*zoom * factor).clamp(SEQ_ZOOM_MIN, SEQ_ZOOM_MAX);
    }

    let row_height = SEQ_BASE_ROW_HEIGHT * *zoom;
    let lifeline_spacing = SEQ_BASE_LIFELINE_SPACING * *zoom;
    let header_height = SEQ_HEADER_HEIGHT;
    let ts_label_margin = SEQ_TS_LABEL_MARGIN;

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

            let total_height = header_height + (num_rows as f32) * row_height + SEQ_BOTTOM_PADDING;

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
            let font_size =
                (SEQ_FONT_SIZE_BASE * *zoom).clamp(SEQ_FONT_SIZE_MIN, SEQ_FONT_SIZE_MAX);
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
            for &x in &lifeline_x {
                let mut y = body_top;
                while y < body_bottom {
                    let y_end = (y + SEQ_LIFELINE_DASH).min(body_bottom);
                    painter.line_segment(
                        [Pos2::new(x, y), Pos2::new(x, y_end)],
                        Stroke::new(SEQ_LIFELINE_STROKE, COLOR_LIFELINE),
                    );
                    y += SEQ_LIFELINE_DASH + SEQ_LIFELINE_GAP;
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
            let ts_font = (SEQ_TS_FONT_BASE * *zoom).clamp(SEQ_TS_FONT_MIN, SEQ_TS_FONT_MAX);
            for (row, &ts) in active_timesteps.iter().enumerate() {
                let y = body_top + (row as f32) * row_height + row_height / 2.0;
                painter.text(
                    Pos2::new(rect.left() + GRID_LABEL_OFFSET, y),
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
                let is_current = current_event.is_some() && msg.record_index == current_event;

                let dot_r = if is_current {
                    SEQ_DOT_RADIUS_CURRENT
                } else {
                    SEQ_DOT_RADIUS
                };

                // Selection ring around the currently selected event
                if is_current {
                    let center = match &msg.kind {
                        MessageKind::Sent => sorted_names
                            .iter()
                            .position(|n| n == &msg.src_node)
                            .map(|i| Pos2::new(lifeline_x[i], y)),
                        MessageKind::Received => sorted_names
                            .iter()
                            .position(|n| n == &msg.src_node)
                            .map(|i| Pos2::new(lifeline_x[i], y)),
                        MessageKind::Dropped(_) => sorted_names
                            .iter()
                            .position(|n| n == &msg.src_node)
                            .map(|i| Pos2::new(lifeline_x[i], y)),
                    };
                    if let Some(center) = center {
                        painter.circle_stroke(
                            center,
                            SEQ_SELECTION_RING_RADIUS,
                            Stroke::new(
                                SEQ_SELECTION_RING_STROKE,
                                egui::Color32::WHITE,
                            ),
                        );
                    }
                }

                match &msg.kind {
                    // ========================================================
                    // TX: draw arrows from sender to every receiver
                    // ========================================================
                    MessageKind::Sent => {
                        let Some(src_x_idx) = sorted_names.iter().position(|n| n == &msg.src_node)
                        else {
                            continue;
                        };
                        let src_x = lifeline_x[src_x_idx];

                        // Sender dot
                        painter.circle_filled(Pos2::new(src_x, y), dot_r, COLOR_TX_OK);

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
                            let base_x = tip.x - dir * SEQ_ARROW_HEAD_LENGTH;
                            painter.add(egui::Shape::convex_polygon(
                                vec![
                                    tip,
                                    Pos2::new(base_x, y - SEQ_ARROW_HEAD_WIDTH),
                                    Pos2::new(base_x, y + SEQ_ARROW_HEAD_WIDTH),
                                ],
                                recv_color,
                                Stroke::NONE,
                            ));

                            // Drop X at destination
                            if is_drop {
                                let half = SEQ_DROP_X_HALF;
                                let stroke = Stroke::new(SEQ_DROP_X_STROKE, recv_color);
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

                            // Tooltip on hover
                            let hover_rect = Rect::from_center_size(
                                Pos2::new(dst_x, y),
                                Vec2::splat(SEQ_HOVER_RECT_SIZE),
                            );
                            if ui.rect_contains_pointer(hover_rect) {
                                let tip_text = match &recv.outcome {
                                    ReceiverOutcome::Dropped(reason) => {
                                        format!("{} dropped: {reason}", recv.node)
                                    }
                                    ReceiverOutcome::Received if recv.has_bit_errors => {
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
                                    ui.id().with(("recv_tip", msg.timestep, &recv.node)),
                                    |ui| {
                                        ui.label(tip_text);
                                    },
                                );
                            }
                        }
                    }

                    // ========================================================
                    // RX: arrow from sender to this node
                    // ========================================================
                    MessageKind::Received => {
                        let Some(rx_idx) = sorted_names.iter().position(|n| n == &msg.src_node)
                        else {
                            continue;
                        };
                        let rx_x = lifeline_x[rx_idx];
                        let thickness = if is_current { 2.5 } else { 1.5 };

                        // Draw arrow from sender to receiver (if sender known)
                        if let Some(ref sender) = msg.dst_node {
                            if let Some(sender_idx) =
                                sorted_names.iter().position(|n| n == sender)
                            {
                                let sender_x = lifeline_x[sender_idx];
                                // Dashed arrow line from sender to receiver
                                let dash_len = 4.0;
                                let gap_len = 3.0;
                                let total_dist = (rx_x - sender_x).abs();
                                let dir = if rx_x > sender_x { 1.0_f32 } else { -1.0 };
                                let mut drawn = 0.0;
                                while drawn < total_dist {
                                    let seg_start = sender_x + dir * drawn;
                                    let seg_end = sender_x
                                        + dir * (drawn + dash_len).min(total_dist);
                                    painter.line_segment(
                                        [Pos2::new(seg_start, y), Pos2::new(seg_end, y)],
                                        Stroke::new(thickness, COLOR_RX),
                                    );
                                    drawn += dash_len + gap_len;
                                }
                                // Arrowhead at receiver
                                let tip = Pos2::new(rx_x, y);
                                let base_x = tip.x - dir * SEQ_ARROW_HEAD_LENGTH;
                                painter.add(egui::Shape::convex_polygon(
                                    vec![
                                        tip,
                                        Pos2::new(base_x, y - SEQ_ARROW_HEAD_WIDTH),
                                        Pos2::new(base_x, y + SEQ_ARROW_HEAD_WIDTH),
                                    ],
                                    COLOR_RX,
                                    Stroke::NONE,
                                ));
                            }
                        }

                        // Filled circle on receiver lifeline
                        painter.circle_filled(Pos2::new(rx_x, y), dot_r, COLOR_RX);
                    }

                    // ========================================================
                    // Drop (standalone): X on the dropping node's lifeline
                    // ========================================================
                    MessageKind::Dropped(reason) => {
                        let Some(idx) = sorted_names.iter().position(|n| n == &msg.src_node)
                        else {
                            continue;
                        };
                        let x = lifeline_x[idx];
                        let half = SEQ_DROP_X_HALF;
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
                        let hover_rect = Rect::from_center_size(
                            Pos2::new(x, y),
                            Vec2::splat(SEQ_HOVER_RECT_SIZE),
                        );
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
                    let src_idx = sorted_names.iter().position(|n| n == &msg.src_node);
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
