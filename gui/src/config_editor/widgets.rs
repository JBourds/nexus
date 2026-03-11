use std::collections::HashSet;
use std::num::{NonZeroU64, NonZeroUsize};

use config::ast::{
    Cmd, DataRate, DataUnit, DistanceUnit, EnergyUnit, PowerFlow, PowerRate, PowerUnit,
    RssiProbExpr, TimeUnit,
};
use egui::Ui;

/// Inline "add item" row: text input + Add button.
/// Returns `Some(name)` when the user confirms and the buffer is non-empty.
pub fn add_item_ui(ui: &mut Ui, label: &str, buf: &mut String) -> Option<String> {
    let mut result = None;
    ui.horizontal(|ui| {
        ui.label(label);
        let resp = ui.text_edit_singleline(buf);
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Add").clicked() || enter) && !buf.is_empty() {
            result = Some(std::mem::take(buf));
        }
    });
    result
}

/// Small red "X" remove button with hover tooltip. Returns `true` when clicked.
pub fn remove_button(ui: &mut Ui) -> bool {
    let text = egui::RichText::new("\u{2716}")
        .color(egui::Color32::from_rgb(200, 60, 60))
        .size(12.0);
    ui.add(egui::Button::new(text).small().frame(false))
        .on_hover_text("Remove")
        .clicked()
}

/// Generic combo box from a slice of `(label, value)` pairs.
pub fn enum_combo<T: Clone + PartialEq>(
    ui: &mut Ui,
    id: &str,
    current: &mut T,
    pairs: &[(&str, T)],
) {
    let current_label = pairs
        .iter()
        .find(|(_, v)| v == current)
        .map(|(l, _)| *l)
        .unwrap_or("?");
    egui::ComboBox::from_id_salt(id)
        .selected_text(current_label)
        .show_ui(ui, |ui| {
            for (label, value) in pairs {
                ui.selectable_value(current, value.clone(), *label);
            }
        });
}

/// Optional NonZeroU64 editor: checkbox + drag value. Unchecked = None.
pub fn optional_nonzero_u64(ui: &mut Ui, label: &str, val: &mut Option<NonZeroU64>) {
    ui.horizontal(|ui| {
        let mut enabled = val.is_some();
        if ui.checkbox(&mut enabled, label).changed() {
            *val = if enabled {
                Some(NonZeroU64::new(1).unwrap())
            } else {
                None
            };
        }
        if let Some(v) = val {
            let mut n = v.get();
            if ui
                .add(egui::DragValue::new(&mut n).range(1..=u64::MAX))
                .changed()
                && let Some(nz) = NonZeroU64::new(n)
            {
                *v = nz;
            }
        }
    });
}

/// Optional NonZeroUsize editor: checkbox + drag value. Unchecked = None.
pub fn optional_nonzero_usize(ui: &mut Ui, label: &str, val: &mut Option<NonZeroUsize>) {
    ui.horizontal(|ui| {
        let mut enabled = val.is_some();
        if ui.checkbox(&mut enabled, label).changed() {
            *val = if enabled {
                Some(NonZeroUsize::new(1).unwrap())
            } else {
                None
            };
        }
        if let Some(v) = val {
            let mut n = v.get();
            if ui
                .add(egui::DragValue::new(&mut n).range(1..=usize::MAX))
                .changed()
                && let Some(nz) = NonZeroUsize::new(n)
            {
                *v = nz;
            }
        }
    });
}

/// Two text inputs: command string + args (comma-separated).
#[allow(dead_code)]
pub fn cmd_editor(ui: &mut Ui, id: &str, cmd: &mut Cmd) {
    ui.horizontal(|ui| {
        ui.label("cmd:");
        ui.add(egui::TextEdit::singleline(&mut cmd.cmd).id(egui::Id::new(format!("{id}_cmd"))));
    });
    ui.horizontal(|ui| {
        ui.label("args:");
        let mut args_str = cmd.args.join(", ");
        if ui
            .add(egui::TextEdit::singleline(&mut args_str).id(egui::Id::new(format!("{id}_args"))))
            .changed()
        {
            cmd.args = args_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    });
}

/// Checkbox per available channel name.
#[allow(dead_code)]
pub fn channel_multi_select(
    ui: &mut Ui,
    id: &str,
    selected: &mut HashSet<String>,
    available: &[String],
) {
    ui.horizontal_wrapped(|ui| {
        for ch in available {
            let mut checked = selected.contains(ch);
            if ui
                .checkbox(&mut checked, ch)
                .on_hover_text(format!("{id}: {ch}"))
                .changed()
            {
                if checked {
                    selected.insert(ch.clone());
                } else {
                    selected.remove(ch);
                }
            }
        }
    });
}

/// DataRate editor: rate DragValue + DataUnit combo + TimeUnit combo.
pub fn data_rate_editor(ui: &mut Ui, id: &str, rate: &mut DataRate) {
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut rate.rate).range(0..=u64::MAX));
        enum_combo(ui, &format!("{id}_du"), &mut rate.data, DATA_UNIT_PAIRS);
        ui.label("/");
        enum_combo(ui, &format!("{id}_tu"), &mut rate.time, TIME_UNIT_PAIRS);
    });
}

/// PowerRate editor: rate DragValue + PowerUnit combo + TimeUnit combo.
#[allow(dead_code)]
pub fn power_rate_editor(ui: &mut Ui, id: &str, rate: &mut PowerRate) {
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut rate.rate));
        enum_combo(ui, &format!("{id}_pu"), &mut rate.unit, POWER_UNIT_PAIRS);
        ui.label("/");
        enum_combo(ui, &format!("{id}_tu"), &mut rate.time, TIME_UNIT_PAIRS);
    });
}

/// RssiProbExpr editor: expression text input + noise floor DragValue.
pub fn rssi_prob_expr_editor(ui: &mut Ui, id: &str, expr: &mut RssiProbExpr) {
    ui.horizontal(|ui| {
        ui.label("expr:");
        ui.add(
            egui::TextEdit::singleline(&mut expr.expr)
                .id(egui::Id::new(format!("{id}_expr")))
                .desired_width(120.0),
        );
        ui.label("noise floor (dBm):");
        dbm_drag_value(ui, &mut expr.noise_floor_dbm);
    });
}

/// DragValue for dBm fields that displays extreme values as +/- infinity.
pub fn dbm_drag_value(ui: &mut Ui, val: &mut f64) {
    let display = format_dbm(*val);
    let mut text = display.clone();
    let response = ui.add(
        egui::TextEdit::singleline(&mut text)
            .desired_width(60.0)
            .horizontal_align(egui::Align::RIGHT),
    );
    if response.changed() {
        let trimmed = text.trim();
        if trimmed == "+inf" || trimmed == "inf" {
            *val = f64::MAX;
        } else if trimmed == "-inf" {
            *val = f64::MIN;
        } else if let Ok(v) = trimmed.parse::<f64>() {
            *val = v;
        }
    }
}

fn format_dbm(val: f64) -> String {
    if val >= f64::MAX / 2.0 {
        "+inf".to_string()
    } else if val <= f64::MIN / 2.0 {
        "-inf".to_string()
    } else {
        format!("{val:.3}")
    }
}

// --- Constant lookup tables for enum combos ---

pub const TIME_UNIT_PAIRS: &[(&str, TimeUnit)] = &[
    ("Hours", TimeUnit::Hours),
    ("Minutes", TimeUnit::Minutes),
    ("Seconds", TimeUnit::Seconds),
    ("Milliseconds", TimeUnit::Milliseconds),
    ("Microseconds", TimeUnit::Microseconds),
    ("Nanoseconds", TimeUnit::Nanoseconds),
];

pub const DATA_UNIT_PAIRS: &[(&str, DataUnit)] = &[
    ("Bit", DataUnit::Bit),
    ("Kilobit", DataUnit::Kilobit),
    ("Megabit", DataUnit::Megabit),
    ("Gigabit", DataUnit::Gigabit),
    ("Byte", DataUnit::Byte),
    ("Kilobyte", DataUnit::Kilobyte),
    ("Megabyte", DataUnit::Megabyte),
    ("Gigabyte", DataUnit::Gigabyte),
];

#[allow(dead_code)]
pub const POWER_UNIT_PAIRS: &[(&str, PowerUnit)] = &[
    ("NanoWatt", PowerUnit::NanoWatt),
    ("MicroWatt", PowerUnit::MicroWatt),
    ("MilliWatt", PowerUnit::MilliWatt),
    ("Watt", PowerUnit::Watt),
    ("KiloWatt", PowerUnit::KiloWatt),
    ("MegaWatt", PowerUnit::MegaWatt),
    ("GigaWatt", PowerUnit::GigaWatt),
];

pub const DISTANCE_UNIT_PAIRS: &[(&str, DistanceUnit)] = &[
    ("Millimeters", DistanceUnit::Millimeters),
    ("Centimeters", DistanceUnit::Centimeters),
    ("Meters", DistanceUnit::Meters),
    ("Kilometers", DistanceUnit::Kilometers),
];

use config::ast::ClockUnit;

#[allow(dead_code)]
pub const CLOCK_UNIT_PAIRS: &[(&str, ClockUnit)] = &[
    ("Hertz", ClockUnit::Hertz),
    ("Kilohertz", ClockUnit::Kilohertz),
    ("Megahertz", ClockUnit::Megahertz),
    ("Gigahertz", ClockUnit::Gigahertz),
];

use config::ast::SignalShape;

pub const SIGNAL_SHAPE_PAIRS: &[(&str, SignalShape)] = &[
    ("Omnidirectional", SignalShape::Omnidirectional),
    ("Cone", SignalShape::Cone),
    ("Direct", SignalShape::Direct),
];

#[allow(dead_code)]
pub const ENERGY_UNIT_PAIRS: &[(&str, EnergyUnit)] = &[
    ("NanoJoule", EnergyUnit::NanoJoule),
    ("MicroJoule", EnergyUnit::MicroJoule),
    ("MilliJoule", EnergyUnit::MilliJoule),
    ("Joule", EnergyUnit::Joule),
    ("KiloJoule", EnergyUnit::KiloJoule),
    ("MicroWattHour", EnergyUnit::MicroWattHour),
    ("MilliWattHour", EnergyUnit::MilliWattHour),
    ("WattHour", EnergyUnit::WattHour),
    ("KiloWattHour", EnergyUnit::KiloWattHour),
];

/// Editor for a `PowerFlow` -- switches between Constant and PiecewiseLinear.
#[allow(dead_code)]
pub fn power_flow_editor(ui: &mut Ui, id: &str, flow: &mut PowerFlow) {
    let is_constant = matches!(flow, PowerFlow::Constant(_));
    let mut mode = if is_constant { 0 } else { 1 };
    ui.horizontal(|ui| {
        ui.label("Mode:");
        egui::ComboBox::from_id_salt(format!("{id}_mode"))
            .selected_text(if mode == 0 {
                "Constant"
            } else {
                "PiecewiseLinear"
            })
            .show_ui(ui, |ui| {
                if ui.selectable_value(&mut mode, 0, "Constant").changed() {
                    *flow = PowerFlow::Constant(PowerRate {
                        rate: 0,
                        unit: Default::default(),
                        time: Default::default(),
                    });
                }
                if ui
                    .selectable_value(&mut mode, 1, "PiecewiseLinear")
                    .changed()
                {
                    *flow = PowerFlow::PiecewiseLinear {
                        unit: Default::default(),
                        time: Default::default(),
                        breakpoints: vec![(0, 0)],
                        repeat_us: None,
                    };
                }
            });
    });

    match flow {
        PowerFlow::Constant(rate) => {
            power_rate_editor(ui, &format!("{id}_const"), rate);
        }
        PowerFlow::PiecewiseLinear {
            unit,
            time,
            breakpoints,
            repeat_us,
        } => {
            ui.horizontal(|ui| {
                ui.label("Unit:");
                enum_combo(ui, &format!("{id}_pu"), unit, POWER_UNIT_PAIRS);
                ui.label("/");
                enum_combo(ui, &format!("{id}_tu"), time, TIME_UNIT_PAIRS);
            });

            ui.label("Breakpoints:");
            let mut to_remove = Vec::new();
            for (i, (t, r)) in breakpoints.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label("t(µs):");
                    ui.add(egui::DragValue::new(t));
                    ui.label("rate:");
                    ui.add(egui::DragValue::new(r));
                    if ui.small_button("✕").clicked() {
                        to_remove.push(i);
                    }
                });
            }
            for i in to_remove.into_iter().rev() {
                breakpoints.remove(i);
            }
            if ui.small_button("+ Breakpoint").clicked() {
                let last_t = breakpoints.last().map(|(t, _)| *t).unwrap_or(0);
                breakpoints.push((last_t + 1_000_000, 0));
            }

            // Optional repeat
            let mut has_repeat = repeat_us.is_some();
            if ui.checkbox(&mut has_repeat, "Repeat period (µs)").changed() {
                *repeat_us = if has_repeat { Some(1_000_000) } else { None };
            }
            if let Some(rep) = repeat_us {
                ui.add(egui::DragValue::new(rep));
            }
        }
    }
}
