use egui::Ui;

use crate::constants::*;
use crate::state::Breakpoint;

pub struct TimelineAction {
    pub seek_to: Option<u64>,
    pub toggle_play: bool,
    pub step_forward: bool,
    pub step_backward: bool,
}

/// Show the timeline panel with playback controls and scrubber.
///
/// `event_stepping` / `event_cursor` / `total_records` enable event-level controls.
/// `breakpoints` renders timestep breakpoint markers on the scrubber.
pub fn show_timeline(
    ui: &mut Ui,
    current_timestep: &mut u64,
    total_timesteps: u64,
    playing: &mut bool,
    playback_speed: &mut f32,
    event_stepping: &mut bool,
    event_cursor: Option<usize>,
    total_records: usize,
    breakpoints: &[Breakpoint],
) -> TimelineAction {
    let mut action = TimelineAction {
        seek_to: None,
        toggle_play: false,
        step_forward: false,
        step_backward: false,
    };

    egui::Frame::NONE.inner_margin(PANEL_FRAME_MARGIN).show(ui, |ui| {
        ui.horizontal(|ui| {
            // Playback controls
            if ui.button("|<").on_hover_text("Jump to start").clicked() {
                action.seek_to = Some(0);
            }
            let step_back_tip = if *event_stepping {
                "Step backward (event)"
            } else {
                "Step backward (timestep)"
            };
            if ui.button("<").on_hover_text(step_back_tip).clicked() {
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
            let step_fwd_tip = if *event_stepping {
                "Step forward (event)"
            } else {
                "Step forward (timestep)"
            };
            if ui.button(">").on_hover_text(step_fwd_tip).clicked() {
                action.step_forward = true;
            }
            if ui.button(">|").on_hover_text("Jump to end").clicked() {
                action.seek_to = Some(total_timesteps.saturating_sub(1));
            }

            ui.separator();

            // Event mode toggle
            let mode_label = if *event_stepping { "Event" } else { "Timestep" };
            if ui
                .selectable_label(*event_stepping, mode_label)
                .on_hover_text("Toggle event-level vs timestep-level stepping")
                .clicked()
            {
                *event_stepping = !*event_stepping;
            }

            ui.separator();

            // Timeline scrubber
            let mut ts_f32 = *current_timestep as f32;
            let max = (total_timesteps.saturating_sub(1)) as f32;
            let slider = egui::Slider::new(&mut ts_f32, 0.0..=max)
                .text("timestep")
                .integer();
            let slider_resp = ui.add(slider);
            if slider_resp.changed() {
                action.seek_to = Some(ts_f32 as u64);
            }

            // Draw breakpoint markers on the slider (small dots above)
            if !breakpoints.is_empty() && max > 0.0 {
                let slider_rect = slider_resp.rect;
                let painter = ui.painter();
                for bp in breakpoints {
                    if !bp.enabled {
                        continue;
                    }
                    if let crate::state::BreakpointKind::Timestep(ts) = &bp.kind {
                        let frac = *ts as f32 / max;
                        let x = slider_rect.left() + frac * slider_rect.width();
                        let y = slider_rect.top() - 3.0;
                        painter.circle_filled(
                            egui::Pos2::new(x, y),
                            3.0,
                            COLOR_BREAKPOINT_ENABLED,
                        );
                    }
                }
            }

            ui.separator();

            // Speed control (only commits on Enter or focus loss to avoid
            // intermediate values while the user is typing)
            ui.label("Speed:");
            ui.add(
                egui::DragValue::new(playback_speed)
                    .speed(PLAYBACK_SPEED_MIN)
                    .range(PLAYBACK_SPEED_MIN..=PLAYBACK_SPEED_MAX)
                    .suffix("x")
                    .update_while_editing(false),
            );

            // Timestep + event display
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if *event_stepping {
                    let ev = event_cursor.map(|c| c + 1).unwrap_or(0);
                    ui.label(format!(
                        "t={} / {}  |  event {} / {}",
                        current_timestep, total_timesteps, ev, total_records
                    ));
                } else {
                    ui.label(format!("t={} / {}", current_timestep, total_timesteps));
                }
            });
        });
    }); // Frame

    action
}
