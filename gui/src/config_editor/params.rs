use std::time::{Duration, SystemTime, UNIX_EPOCH};

use config::ast::Params;
use egui::Ui;

use super::widgets::{TIME_UNIT_PAIRS, enum_combo};

pub fn show_params(ui: &mut Ui, params: &mut Params) {
    ui.horizontal(|ui| {
        ui.label("Timestep length:");
        let mut len = params.timestep.length.get();
        if ui
            .add(egui::DragValue::new(&mut len).range(1..=u64::MAX))
            .changed()
            && let Some(v) = std::num::NonZeroU64::new(len)
        {
            params.timestep.length = v;
        }
    });

    ui.horizontal(|ui| {
        ui.label("Unit:");
        enum_combo(ui, "ts_unit", &mut params.timestep.unit, TIME_UNIT_PAIRS);
    });

    ui.horizontal(|ui| {
        ui.label("Timestep count:");
        let mut count = params.timestep.count.get();
        if ui
            .add(egui::DragValue::new(&mut count).range(1..=u64::MAX))
            .changed()
            && let Some(v) = std::num::NonZeroU64::new(count)
        {
            params.timestep.count = v;
        }
    });

    // Start time as ISO-8601 text input
    ui.horizontal(|ui| {
        ui.label("Start time:");
        let dur = params
            .timestep
            .start
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let secs = dur.as_secs() as i64;
        let nanos = dur.subsec_nanos();
        // Format as RFC-3339 (UTC)
        let dt_secs = secs;
        let (y, mo, d, h, mi, s) = epoch_to_utc(dt_secs);
        let mut text = if nanos == 0 {
            format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
        } else {
            format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{nanos:09}Z")
        };
        let resp = ui.add(egui::TextEdit::singleline(&mut text).desired_width(240.0));
        if resp.changed()
            && let Some(st) = parse_rfc3339(&text)
        {
            params.timestep.start = st;
        }
    });

    ui.horizontal(|ui| {
        ui.label("Seed:");
        ui.add(egui::DragValue::new(&mut params.seed));
    });

    ui.horizontal(|ui| {
        ui.label("Time dilation:");
        ui.add(
            egui::DragValue::new(&mut params.time_dilation)
                .speed(0.01)
                .range(0.001..=1000.0),
        );
    });

    ui.horizontal(|ui| {
        ui.label("Root:");
        let mut root_str = params.root.to_string_lossy().to_string();
        if ui.text_edit_singleline(&mut root_str).changed() {
            params.root = root_str.into();
        }
    });
}

/// Simple RFC-3339 parser for `YYYY-MM-DDThh:mm:ssZ` or with fractional seconds.
fn parse_rfc3339(s: &str) -> Option<SystemTime> {
    let s = s.trim();
    // Expect at least "YYYY-MM-DDThh:mm:ssZ"
    if s.len() < 20 {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: u32 = s.get(5..7)?.parse().ok()?;
    let day: u32 = s.get(8..10)?.parse().ok()?;
    let hour: u32 = s.get(11..13)?.parse().ok()?;
    let min: u32 = s.get(14..16)?.parse().ok()?;
    let sec: u32 = s.get(17..19)?.parse().ok()?;

    let mut nanos: u32 = 0;
    let rest = &s[19..];
    let rest = if let Some(frac) = rest.strip_prefix('.') {
        let end = frac
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(frac.len());
        let frac_str = &frac[..end];
        if !frac_str.is_empty() {
            let padded = format!("{frac_str:0<9}");
            nanos = padded[..9].parse().ok()?;
        }
        &frac[end..]
    } else {
        rest
    };
    // Must end with 'Z' (UTC only for simplicity)
    if !rest.eq_ignore_ascii_case("z") {
        return None;
    }

    let epoch_days = days_from_civil(year, month, day)?;
    let epoch_secs = epoch_days as i64 * 86400 + hour as i64 * 3600 + min as i64 * 60 + sec as i64;
    if epoch_secs < 0 {
        return None;
    }
    Some(UNIX_EPOCH + Duration::new(epoch_secs as u64, nanos))
}

/// Convert epoch seconds to UTC (y, m, d, h, min, sec).
fn epoch_to_utc(epoch_secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let secs_in_day: i64 = 86400;
    let mut days = epoch_secs.div_euclid(secs_in_day);
    let day_secs = epoch_secs.rem_euclid(secs_in_day) as u32;
    let h = day_secs / 3600;
    let mi = (day_secs % 3600) / 60;
    let s = day_secs % 60;

    // Civil from days (algorithm from Howard Hinnant)
    days += 719468;
    let era = days.div_euclid(146097);
    let doe = days.rem_euclid(146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, h, mi, s)
}

/// Days from civil date to Unix epoch (algorithm from Howard Hinnant).
fn days_from_civil(y: i64, m: u32, d: u32) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe as i64 - 719468)
}
