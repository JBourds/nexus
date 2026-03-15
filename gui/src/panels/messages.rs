use std::collections::HashSet;

use egui::Ui;

use crate::constants::*;
use crate::state::{MessageEntry, MessageKind, ReceiverOutcome};

/// Action from the messages panel that the caller should handle.
pub enum MessagesAction {
    None,
    /// User clicked a receiver node name; select it on the grid.
    SelectNode(String),
    /// User wants to jump to a specific event index.
    JumpToEvent(usize),
}

/// Show the message list panel.
///
/// `current_event_index` highlights the row whose `record_index` matches (event-stepping mode).
/// `expanded_messages` tracks which TX rows are expanded to show receivers.
pub fn show_messages(
    ui: &mut Ui,
    messages: &[MessageEntry],
    max_display: usize,
    current_event_index: Option<usize>,
    expanded_messages: &mut HashSet<usize>,
) -> MessagesAction {
    let mut action = MessagesAction::None;

    egui::Frame::NONE
        .inner_margin(PANEL_FRAME_MARGIN)
        .show(ui, |ui| {
            if messages.is_empty() {
                ui.label("No messages yet");
                return;
            }

            let start = messages.len().saturating_sub(max_display);
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for (msg_idx, msg) in messages[start..].iter().enumerate() {
                        let absolute_idx = start + msg_idx;
                        let icon = match &msg.kind {
                            MessageKind::Sent => "TX",
                            MessageKind::Received => "RX",
                            MessageKind::Dropped(_) => "XX",
                        };
                        let color = match &msg.kind {
                            MessageKind::Sent => COLOR_TX_OK,
                            MessageKind::Received => COLOR_RX,
                            MessageKind::Dropped(_) => COLOR_DROP,
                        };

                        // Highlight the current event row
                        let is_current = current_event_index.is_some()
                            && msg.record_index.is_some()
                            && msg.record_index == current_event_index;

                        let frame = if is_current {
                            egui::Frame::NONE
                                .fill(COLOR_EVENT_HIGHLIGHT)
                                .inner_margin(2.0)
                                .corner_radius(2.0)
                        } else {
                            egui::Frame::NONE
                        };

                        frame.show(ui, |ui| {
                            let resp = ui.horizontal(|ui| {
                                ui.colored_label(color, format!("[{}]", icon));
                                ui.label(format!("t={}", msg.timestep));
                                ui.label(&msg.src_node);
                                if let Some(dst) = &msg.dst_node {
                                    ui.label(format!("-> {dst}"));
                                }
                                ui.label(&msg.channel);

                                // TX expand toggle
                                if msg.kind == MessageKind::Sent && !msg.receivers.is_empty() {
                                    let is_expanded = expanded_messages.contains(&absolute_idx);
                                    let expand_label =
                                        if is_expanded { "\u{25bc}" } else { "\u{25b6}" };
                                    if ui
                                        .small_button(expand_label)
                                        .on_hover_text("Show receivers")
                                        .clicked()
                                    {
                                        if is_expanded {
                                            expanded_messages.remove(&absolute_idx);
                                        } else {
                                            expanded_messages.insert(absolute_idx);
                                        }
                                    }
                                }

                                if !msg.data_raw.is_empty()
                                    && ui
                                        .small_button("\u{2398}")
                                        .on_hover_text("Copy to clipboard")
                                        .clicked()
                                {
                                    ui.ctx().copy_text(msg.data_preview.clone());
                                }
                            });

                            // Click row to jump to this event
                            if resp.response.clicked()
                                && let Some(rec_idx) = msg.record_index {
                                    action = MessagesAction::JumpToEvent(rec_idx);
                                }

                            if !msg.data_preview.is_empty() {
                                ui.indent(msg.timestep, |ui| {
                                    ui.label(
                                        egui::RichText::new(&msg.data_preview).monospace().small(),
                                    );
                                });
                            }

                            // Expanded receiver list for TX messages
                            if msg.kind == MessageKind::Sent
                                && expanded_messages.contains(&absolute_idx)
                            {
                                ui.indent(ui.id().with(("receivers", absolute_idx)), |ui| {
                                    for recv in &msg.receivers {
                                        ui.horizontal(|ui| {
                                            match &recv.outcome {
                                                ReceiverOutcome::Received => {
                                                    ui.colored_label(COLOR_TX_OK, "\u{2713}");
                                                }
                                                ReceiverOutcome::Dropped(reason) => {
                                                    ui.colored_label(COLOR_DROP, "\u{2717}");
                                                    ui.label(
                                                        egui::RichText::new(reason)
                                                            .small()
                                                            .color(COLOR_DROP),
                                                    );
                                                }
                                            }
                                            let node_resp = ui.link(&recv.node);
                                            if node_resp.clicked() {
                                                action =
                                                    MessagesAction::SelectNode(recv.node.clone());
                                            }
                                        });
                                    }
                                });
                            }
                        });
                    }
                });
        }); // Frame

    action
}
