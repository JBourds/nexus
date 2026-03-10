use config::ast::{self, DistanceTimeVar, Medium, SignalShape};
use egui::Ui;

use super::widgets::{
    DISTANCE_UNIT_PAIRS, SIGNAL_SHAPE_PAIRS, TIME_UNIT_PAIRS, data_rate_editor, dbm_drag_value,
    enum_combo, rssi_prob_expr_editor,
};

pub fn show_link(ui: &mut Ui, id: &str, link: &mut ast::Link) {
    // Medium switcher
    let is_wireless = matches!(link.medium, Medium::Wireless { .. });
    let mut medium_idx: usize = if is_wireless { 0 } else { 1 };
    ui.horizontal(|ui| {
        ui.label("Medium:");
        egui::ComboBox::from_id_salt(format!("{id}_medium"))
            .selected_text(if is_wireless { "Wireless" } else { "Wired" })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut medium_idx, 0, "Wireless");
                ui.selectable_value(&mut medium_idx, 1, "Wired");
            });
    });

    // Switch medium type if changed
    if medium_idx == 0 && !is_wireless {
        link.medium = Medium::Wireless {
            shape: SignalShape::Omnidirectional,
            wavelength_meters: 0.125,
            gain_dbi: 2.0,
            rx_min_dbm: -120.0,
            tx_min_dbm: -10.0,
            tx_max_dbm: 20.0,
        };
    } else if medium_idx == 1 && is_wireless {
        link.medium = Medium::default();
    }

    match &mut link.medium {
        Medium::Wireless {
            shape,
            wavelength_meters,
            gain_dbi,
            rx_min_dbm,
            tx_min_dbm,
            tx_max_dbm,
        } => {
            ui.horizontal(|ui| {
                ui.label("Shape:");
                enum_combo(ui, &format!("{id}_shape"), shape, SIGNAL_SHAPE_PAIRS);
            });
            ui.horizontal(|ui| {
                ui.label("Wavelength (m):");
                ui.add(egui::DragValue::new(wavelength_meters).speed(0.001));
            });
            ui.horizontal(|ui| {
                ui.label("Gain (dBi):");
                ui.add(egui::DragValue::new(gain_dbi).speed(0.1));
            });
            ui.horizontal(|ui| {
                ui.label("RX min (dBm):");
                dbm_drag_value(ui, rx_min_dbm);
            });
            ui.horizontal(|ui| {
                ui.label("TX min (dBm):");
                dbm_drag_value(ui, tx_min_dbm);
            });
            ui.horizontal(|ui| {
                ui.label("TX max (dBm):");
                dbm_drag_value(ui, tx_max_dbm);
            });
        }
        Medium::Wired {
            rx_min_dbm,
            tx_min_dbm,
            tx_max_dbm,
            r,
            l,
            c,
            g,
            f,
        } => {
            ui.horizontal(|ui| {
                ui.label("RX min (dBm):");
                dbm_drag_value(ui, rx_min_dbm);
            });
            ui.horizontal(|ui| {
                ui.label("TX (dBm):");
                dbm_drag_value(ui, tx_min_dbm);
                ui.label("-");
                dbm_drag_value(ui, tx_max_dbm);
            });
            ui.horizontal(|ui| {
                ui.label("R:");
                ui.add(egui::DragValue::new(r).speed(0.001));
                ui.label("L:");
                ui.add(egui::DragValue::new(l).speed(1e-9));
            });
            ui.horizontal(|ui| {
                ui.label("C:");
                ui.add(egui::DragValue::new(c).speed(1e-12));
                ui.label("G:");
                ui.add(egui::DragValue::new(g).speed(1e-6));
            });
            ui.horizontal(|ui| {
                ui.label("Freq (Hz):");
                ui.add(egui::DragValue::new(f).speed(1000.0));
            });
        }
    }

    // Delays subsection
    ui.separator();
    ui.label("Delays:");
    ui.indent(format!("{id}_delays"), |ui| {
        ui.label("Transmission:");
        data_rate_editor(ui, &format!("{id}_tx_rate"), &mut link.delays.transmission);

        ui.label("Processing:");
        data_rate_editor(ui, &format!("{id}_proc_rate"), &mut link.delays.processing);

        ui.label("Propagation:");
        show_distance_time_var(ui, &format!("{id}_prop"), &mut link.delays.propagation);
    });

    // Error models subsection
    ui.separator();
    ui.label("Error Models:");
    ui.indent(format!("{id}_errors"), |ui| {
        ui.label("Bit Error:");
        rssi_prob_expr_editor(ui, &format!("{id}_bit_err"), &mut link.bit_error);

        ui.label("Packet Loss:");
        rssi_prob_expr_editor(ui, &format!("{id}_pkt_loss"), &mut link.packet_loss);
    });
}

fn show_distance_time_var(ui: &mut Ui, id: &str, dtv: &mut DistanceTimeVar) {
    ui.horizontal(|ui| {
        ui.label("rate expr:");
        ui.add(
            egui::TextEdit::singleline(&mut dtv.rate)
                .id(egui::Id::new(format!("{id}_rate")))
                .desired_width(100.0),
        );
        enum_combo(ui, &format!("{id}_tu"), &mut dtv.time, TIME_UNIT_PAIRS);
        ui.label("/");
        enum_combo(
            ui,
            &format!("{id}_du"),
            &mut dtv.distance,
            DISTANCE_UNIT_PAIRS,
        );
    });
}
