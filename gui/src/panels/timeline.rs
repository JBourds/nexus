use egui::Ui;

pub struct TimelineAction {
    pub seek_to: Option<u64>,
    pub toggle_play: bool,
    pub step_forward: bool,
    pub step_backward: bool,
}

/// Show the timeline panel with playback controls and scrubber.
pub fn show_timeline(
    ui: &mut Ui,
    current_timestep: &mut u64,
    total_timesteps: u64,
    playing: &mut bool,
    playback_speed: &mut f32,
) -> TimelineAction {
    let mut action = TimelineAction {
        seek_to: None,
        toggle_play: false,
        step_forward: false,
        step_backward: false,
    };

    egui::Frame::NONE.inner_margin(6.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            // Playback controls
            if ui.button("|<").on_hover_text("Jump to start").clicked() {
                action.seek_to = Some(0);
            }
            if ui.button("<").on_hover_text("Step backward").clicked() {
                action.step_backward = true;
            }
            let play_label = if *playing { "||" } else { ">" };
            if ui
                .button(play_label)
                .on_hover_text(if *playing { "Pause" } else { "Play" })
                .clicked()
            {
                action.toggle_play = true;
            }
            if ui.button(">").on_hover_text("Step forward").clicked() {
                action.step_forward = true;
            }
            if ui.button(">|").on_hover_text("Jump to end").clicked() {
                action.seek_to = Some(total_timesteps.saturating_sub(1));
            }

            ui.separator();

            // Timeline scrubber
            let mut ts_f32 = *current_timestep as f32;
            let max = (total_timesteps.saturating_sub(1)) as f32;
            let slider = egui::Slider::new(&mut ts_f32, 0.0..=max)
                .text("timestep")
                .integer();
            if ui.add(slider).changed() {
                action.seek_to = Some(ts_f32 as u64);
            }

            ui.separator();

            // Speed control
            ui.label("Speed:");
            ui.add(
                egui::DragValue::new(playback_speed)
                    .speed(0.1)
                    .range(0.1..=10.0)
                    .suffix("x"),
            );

            // Timestep display
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("t={} / {}", current_timestep, total_timesteps));
            });
        });
    }); // Frame

    action
}
