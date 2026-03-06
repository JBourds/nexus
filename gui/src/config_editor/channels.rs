use std::num::NonZeroUsize;

use config::ast::{self, ChannelType, TimeUnit};
use egui::Ui;

use super::links;
use super::widgets::{
    TIME_UNIT_PAIRS, add_item_ui, enum_combo, optional_nonzero_u64, optional_nonzero_usize,
    remove_button,
};

pub fn show_channels(ui: &mut Ui, sim: &mut ast::Simulation, buf: &mut String) {
    if let Some(name) = add_item_ui(ui, "+ Channel:", buf)
        && !sim.channels.contains_key(&name)
    {
        sim.channels.insert(name, ast::Channel::default());
    }

    let mut to_remove = Vec::new();
    let channel_names: Vec<String> = {
        let mut names: Vec<_> = sim.channels.keys().cloned().collect();
        names.sort();
        names
    };

    for name in &channel_names {
        let id = ui.make_persistent_id(format!("ch_{name}"));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
            .show_header(ui, |ui| {
                ui.label(name);
                if remove_button(ui) {
                    to_remove.push(name.clone());
                }
            })
            .body(|ui| {
                if let Some(channel) = sim.channels.get_mut(name) {
                    show_channel_body(ui, name, channel);
                }
            });
    }

    for name in to_remove {
        sim.channels.remove(&name);
    }
}

fn show_channel_body(ui: &mut Ui, name: &str, channel: &mut ast::Channel) {
    // Type switcher
    let is_shared = matches!(channel.r#type, ChannelType::Shared { .. });
    let mut type_idx: usize = if is_shared { 0 } else { 1 };
    ui.horizontal(|ui| {
        ui.label("Type:");
        egui::ComboBox::from_id_salt(format!("ch_type_{name}"))
            .selected_text(if is_shared { "Shared" } else { "Exclusive" })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut type_idx, 0, "Shared");
                ui.selectable_value(&mut type_idx, 1, "Exclusive");
            });
    });

    // Switch type if changed
    if type_idx == 0 && !is_shared {
        channel.r#type = ChannelType::Shared {
            ttl: None,
            unit: TimeUnit::Seconds,
            read_own_writes: false,
            max_size: NonZeroUsize::new(256).unwrap(),
        };
    } else if type_idx == 1 && is_shared {
        channel.r#type = ChannelType::default();
    }

    match &mut channel.r#type {
        ChannelType::Shared {
            ttl,
            unit,
            read_own_writes,
            max_size,
        } => {
            optional_nonzero_u64(ui, "TTL:", ttl);
            if ttl.is_some() {
                ui.horizontal(|ui| {
                    ui.label("  TTL unit:");
                    enum_combo(ui, &format!("ch_ttlu_{name}"), unit, TIME_UNIT_PAIRS);
                });
            }
            ui.horizontal(|ui| {
                ui.label("Max size:");
                let mut sz = max_size.get();
                if ui
                    .add(egui::DragValue::new(&mut sz).range(1..=usize::MAX))
                    .changed()
                    && let Some(v) = NonZeroUsize::new(sz)
                {
                    *max_size = v;
                }
            });
            ui.checkbox(read_own_writes, "Read own writes");
        }
        ChannelType::Exclusive {
            ttl,
            unit,
            max_size,
            nbuffered,
            read_own_writes,
        } => {
            optional_nonzero_u64(ui, "TTL:", ttl);
            if ttl.is_some() {
                ui.horizontal(|ui| {
                    ui.label("  TTL unit:");
                    enum_combo(ui, &format!("ch_ttlu_{name}"), unit, TIME_UNIT_PAIRS);
                });
            }
            ui.horizontal(|ui| {
                ui.label("Max size:");
                let mut sz = max_size.get();
                if ui
                    .add(egui::DragValue::new(&mut sz).range(1..=usize::MAX))
                    .changed()
                    && let Some(v) = NonZeroUsize::new(sz)
                {
                    *max_size = v;
                }
            });
            optional_nonzero_usize(ui, "Buffered:", nbuffered);
            ui.checkbox(read_own_writes, "Read own writes");
        }
    }

    // Inline link editor
    ui.separator();
    ui.label("Link:");
    ui.indent(format!("ch_link_{name}"), |ui| {
        links::show_link(ui, &format!("ch_link_{name}"), &mut channel.link);
    });
}
