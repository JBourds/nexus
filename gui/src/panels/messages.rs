use egui::Ui;

use crate::state::{MessageEntry, MessageKind};

/// Show the message list panel.
pub fn show_messages(ui: &mut Ui, messages: &[MessageEntry], max_display: usize) {
    ui.heading("Messages");
    ui.separator();

    if messages.is_empty() {
        ui.label("No messages yet");
        return;
    }

    let start = messages.len().saturating_sub(max_display);
    egui::ScrollArea::vertical().show(ui, |ui| {
        for msg in &messages[start..] {
            let icon = match &msg.kind {
                MessageKind::Sent => "TX",
                MessageKind::Received => "RX",
                MessageKind::Dropped(_) => "XX",
            };
            let color = match &msg.kind {
                MessageKind::Sent => egui::Color32::from_rgb(100, 200, 100),
                MessageKind::Received => egui::Color32::from_rgb(100, 150, 255),
                MessageKind::Dropped(_) => egui::Color32::from_rgb(255, 100, 100),
            };

            ui.horizontal(|ui| {
                ui.colored_label(color, format!("[{}]", icon));
                ui.label(format!("t={}", msg.timestep));
                ui.label(&msg.src_node);
                if let Some(dst) = &msg.dst_node {
                    ui.label(format!("-> {dst}"));
                }
                ui.label(&msg.channel);
            });

            if !msg.data_preview.is_empty() {
                ui.indent(msg.timestep, |ui| {
                    ui.small(&msg.data_preview);
                });
            }
        }
    });
}
