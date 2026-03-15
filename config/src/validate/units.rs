//! Unit validation: trait-based system for parsing unit strings.
//!
//! Each unit type implements [`ValidateUnit`] which tries a case-sensitive
//! abbreviation first, then falls back to case-insensitive full-word matching.

use crate::ast::*;
use crate::parse;
use anyhow::{Context, Result, bail};

/// Validate a unit string by trying case-sensitive abbreviation first, then
/// falling back to case-insensitive full-word matching. This avoids the
/// previous inconsistency where different unit types used three different
/// case-normalization strategies.
pub(crate) trait ValidateUnit: Sized {
    /// Match case-sensitive abbreviations (e.g., "mW" vs "MW").
    /// Return `None` to fall through to case-insensitive matching.
    fn from_abbrev(_s: &str) -> Option<Self> {
        None
    }
    /// Match fully lowercased name or abbreviation.
    fn from_lowercase(s: &str) -> Option<Self>;
    /// Human-readable unit kind for error messages.
    fn unit_kind() -> &'static str;

    fn validate(val: parse::Unit) -> Result<Self> {
        if let Some(v) = Self::from_abbrev(&val.0) {
            return Ok(v);
        }
        let lower = val.0.to_ascii_lowercase();
        Self::from_lowercase(&lower).ok_or_else(|| {
            anyhow::anyhow!(
                "Expected a valid {} but found \"{}\"",
                Self::unit_kind(),
                val.0
            )
        })
    }
}

impl ValidateUnit for ClockUnit {
    fn from_lowercase(s: &str) -> Option<Self> {
        match s {
            "hertz" | "hz" => Some(Self::Hertz),
            "kilohertz" | "khz" => Some(Self::Kilohertz),
            "megahertz" | "mhz" => Some(Self::Megahertz),
            "gigahertz" | "ghz" => Some(Self::Gigahertz),
            _ => None,
        }
    }
    fn unit_kind() -> &'static str {
        "clock unit"
    }
}

impl DataUnit {
    pub(crate) fn validate_byte_aligned(val: parse::Unit) -> Result<Self> {
        let lower = val.0.to_ascii_lowercase();
        match lower.as_str() {
            "bytes" | "byte" => Ok(Self::Byte),
            "kilobytes" | "kilobyte" | "kb" => Ok(Self::Kilobyte),
            "megabytes" | "megabyte" | "mb" => Ok(Self::Megabyte),
            "gigabytes" | "gigabyte" | "gb" => Ok(Self::Gigabyte),
            // Case-sensitive single-char: "B" = bytes
            _ if val.0 == "B" => Ok(Self::Byte),
            _ => bail!(
                "Expected a valid byte-aligned data unit but found \"{}\"",
                val.0
            ),
        }
    }
}

impl ValidateUnit for DataUnit {
    fn from_abbrev(s: &str) -> Option<Self> {
        // Case-sensitive abbreviations to distinguish bits from bytes:
        // lowercase = bits, uppercase final = bytes
        match s {
            "b" => Some(Self::Bit),
            "B" => Some(Self::Byte),
            "kb" => Some(Self::Kilobit),
            "kB" | "KB" => Some(Self::Kilobyte),
            "mb" => Some(Self::Megabit),
            "mB" | "MB" => Some(Self::Megabyte),
            "gb" => Some(Self::Gigabit),
            "gB" | "GB" => Some(Self::Gigabyte),
            _ => None,
        }
    }
    fn from_lowercase(s: &str) -> Option<Self> {
        match s {
            "bits" | "bit" => Some(Self::Bit),
            "kilobits" | "kilobit" => Some(Self::Kilobit),
            "megabits" | "megabit" => Some(Self::Megabit),
            "gigabits" | "gigabit" => Some(Self::Gigabit),
            "bytes" | "byte" => Some(Self::Byte),
            "kilobytes" | "kilobyte" => Some(Self::Kilobyte),
            "megabytes" | "megabyte" => Some(Self::Megabyte),
            "gigabytes" | "gigabyte" => Some(Self::Gigabyte),
            _ => None,
        }
    }
    fn unit_kind() -> &'static str {
        "data unit"
    }
}

impl ValidateUnit for EnergyUnit {
    fn from_lowercase(s: &str) -> Option<Self> {
        match s {
            "nanojoule" | "nanojoules" | "nj" => Some(Self::NanoJoule),
            "microjoule" | "microjoules" | "uj" => Some(Self::MicroJoule),
            "millijoule" | "millijoules" | "mj" => Some(Self::MilliJoule),
            "joule" | "joules" | "j" => Some(Self::Joule),
            "kilojoule" | "kilojoules" | "kj" => Some(Self::KiloJoule),
            "microwatthour" | "microwatthours" | "uwh" => Some(Self::MicroWattHour),
            "milliwatthour" | "milliwatthours" | "mwh" => Some(Self::MilliWattHour),
            "watthour" | "watthours" | "wh" => Some(Self::WattHour),
            "kilowatthour" | "kilowatthours" | "kwh" => Some(Self::KiloWattHour),
            _ => None,
        }
    }
    fn unit_kind() -> &'static str {
        "energy unit"
    }
}

impl ValidateUnit for PowerUnit {
    fn from_abbrev(s: &str) -> Option<Self> {
        // SI convention: case-sensitive prefix distinguishes milli- from Mega-
        match s {
            "nW" | "nw" => Some(Self::NanoWatt),
            "uW" | "uw" => Some(Self::MicroWatt),
            "mW" | "mw" => Some(Self::MilliWatt),
            "W" | "w" => Some(Self::Watt),
            "kW" | "kw" => Some(Self::KiloWatt),
            "MW" => Some(Self::MegaWatt),
            "GW" | "gw" => Some(Self::GigaWatt),
            _ => None,
        }
    }
    fn from_lowercase(s: &str) -> Option<Self> {
        match s {
            "nanowatt" | "nanowatts" => Some(Self::NanoWatt),
            "microwatt" | "microwatts" => Some(Self::MicroWatt),
            "milliwatt" | "milliwatts" => Some(Self::MilliWatt),
            "watt" | "watts" => Some(Self::Watt),
            "kilowatt" | "kilowatts" => Some(Self::KiloWatt),
            "megawatt" | "megawatts" => Some(Self::MegaWatt),
            "gigawatt" | "gigawatts" => Some(Self::GigaWatt),
            _ => None,
        }
    }
    fn unit_kind() -> &'static str {
        "power unit"
    }
}

impl ValidateUnit for TimeUnit {
    fn from_lowercase(s: &str) -> Option<Self> {
        match s {
            "hours" | "h" => Some(Self::Hours),
            "minutes" | "m" => Some(Self::Minutes),
            "seconds" | "s" => Some(Self::Seconds),
            "milliseconds" | "ms" => Some(Self::Milliseconds),
            "microseconds" | "us" => Some(Self::Microseconds),
            "nanoseconds" | "ns" => Some(Self::Nanoseconds),
            _ => None,
        }
    }
    fn unit_kind() -> &'static str {
        "time unit"
    }
}

impl ValidateUnit for DistanceUnit {
    fn from_lowercase(s: &str) -> Option<Self> {
        match s {
            "millimeters" | "mm" => Some(Self::Millimeters),
            "centimeters" | "cm" => Some(Self::Centimeters),
            "meters" | "m" => Some(Self::Meters),
            "kilometers" | "km" => Some(Self::Kilometers),
            _ => None,
        }
    }
    fn unit_kind() -> &'static str {
        "distance unit"
    }
}

/// Validate an optional field, returning the default on `None`.
pub(super) fn validate_optional<V, T: Default>(
    val: Option<V>,
    validator: fn(V) -> Result<T>,
) -> Result<T> {
    val.map(validator).unwrap_or(Ok(T::default()))
}

impl DataRate {
    pub(super) fn validate(val: parse::Rate) -> Result<Self> {
        let data = validate_optional(val.data, DataUnit::validate)
            .context("Unable to validate rate's data unit")?;
        let time = validate_optional(val.time, TimeUnit::validate)
            .context("Unable to validate rate's time unit")?;
        let rate = val.rate.unwrap_or(i64::MAX as u64);
        Ok(Self { rate, data, time })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn u(s: &str) -> parse::Unit {
        parse::Unit(s.to_string())
    }

    // ClockUnit
    #[test]
    fn clock_lowercase() {
        assert!(matches!(
            ClockUnit::validate(u("hertz")).unwrap(),
            ClockUnit::Hertz
        ));
        assert!(matches!(
            ClockUnit::validate(u("kilohertz")).unwrap(),
            ClockUnit::Kilohertz
        ));
    }
    #[test]
    fn clock_abbrev() {
        assert!(matches!(
            ClockUnit::validate(u("Hz")).unwrap(),
            ClockUnit::Hertz
        ));
        assert!(matches!(
            ClockUnit::validate(u("GHz")).unwrap(),
            ClockUnit::Gigahertz
        ));
    }
    #[test]
    fn clock_mixed_case() {
        assert!(matches!(
            ClockUnit::validate(u("HERTZ")).unwrap(),
            ClockUnit::Hertz
        ));
        assert!(matches!(
            ClockUnit::validate(u("Megahertz")).unwrap(),
            ClockUnit::Megahertz
        ));
    }
    #[test]
    fn clock_unknown() {
        assert!(ClockUnit::validate(u("parsec")).is_err());
    }

    // DataUnit
    #[test]
    fn data_full_names() {
        assert!(matches!(
            DataUnit::validate(u("bits")).unwrap(),
            DataUnit::Bit
        ));
        assert!(matches!(
            DataUnit::validate(u("bytes")).unwrap(),
            DataUnit::Byte
        ));
        assert!(matches!(
            DataUnit::validate(u("kilobits")).unwrap(),
            DataUnit::Kilobit
        ));
        assert!(matches!(
            DataUnit::validate(u("megabytes")).unwrap(),
            DataUnit::Megabyte
        ));
    }
    #[test]
    fn data_case_sensitive_abbrev() {
        assert!(matches!(DataUnit::validate(u("b")).unwrap(), DataUnit::Bit));
        assert!(matches!(
            DataUnit::validate(u("B")).unwrap(),
            DataUnit::Byte
        ));
        assert!(matches!(
            DataUnit::validate(u("kb")).unwrap(),
            DataUnit::Kilobit
        ));
        assert!(matches!(
            DataUnit::validate(u("kB")).unwrap(),
            DataUnit::Kilobyte
        ));
        assert!(matches!(
            DataUnit::validate(u("MB")).unwrap(),
            DataUnit::Megabyte
        ));
    }
    #[test]
    fn data_mixed_case() {
        assert!(matches!(
            DataUnit::validate(u("BITS")).unwrap(),
            DataUnit::Bit
        ));
        assert!(matches!(
            DataUnit::validate(u("Megabyte")).unwrap(),
            DataUnit::Megabyte
        ));
    }
    #[test]
    fn data_unknown() {
        assert!(DataUnit::validate(u("nibble")).is_err());
    }

    // TimeUnit
    #[test]
    fn time_full_names() {
        assert!(matches!(
            TimeUnit::validate(u("seconds")).unwrap(),
            TimeUnit::Seconds
        ));
        assert!(matches!(
            TimeUnit::validate(u("microseconds")).unwrap(),
            TimeUnit::Microseconds
        ));
    }
    #[test]
    fn time_abbrevs() {
        assert!(matches!(
            TimeUnit::validate(u("ms")).unwrap(),
            TimeUnit::Milliseconds
        ));
        assert!(matches!(
            TimeUnit::validate(u("ns")).unwrap(),
            TimeUnit::Nanoseconds
        ));
    }
    #[test]
    fn time_case_insensitive() {
        assert!(matches!(
            TimeUnit::validate(u("HOURS")).unwrap(),
            TimeUnit::Hours
        ));
    }

    // DistanceUnit
    #[test]
    fn distance_names() {
        assert!(matches!(
            DistanceUnit::validate(u("meters")).unwrap(),
            DistanceUnit::Meters
        ));
    }
    #[test]
    fn distance_abbrevs() {
        assert!(matches!(
            DistanceUnit::validate(u("km")).unwrap(),
            DistanceUnit::Kilometers
        ));
        assert!(matches!(
            DistanceUnit::validate(u("mm")).unwrap(),
            DistanceUnit::Millimeters
        ));
    }
    #[test]
    fn distance_case_insensitive() {
        assert!(matches!(
            DistanceUnit::validate(u("METERS")).unwrap(),
            DistanceUnit::Meters
        ));
    }

    // EnergyUnit
    #[test]
    fn energy_joules() {
        assert!(matches!(
            EnergyUnit::validate(u("nanojoule")).unwrap(),
            EnergyUnit::NanoJoule
        ));
        assert!(matches!(
            EnergyUnit::validate(u("millijoules")).unwrap(),
            EnergyUnit::MilliJoule
        ));
        assert!(matches!(
            EnergyUnit::validate(u("joule")).unwrap(),
            EnergyUnit::Joule
        ));
    }
    #[test]
    fn energy_watt_hours() {
        assert!(matches!(
            EnergyUnit::validate(u("watthour")).unwrap(),
            EnergyUnit::WattHour
        ));
        assert!(matches!(
            EnergyUnit::validate(u("kwh")).unwrap(),
            EnergyUnit::KiloWattHour
        ));
    }
    #[test]
    fn energy_case_insensitive() {
        assert!(matches!(
            EnergyUnit::validate(u("NANOJOULE")).unwrap(),
            EnergyUnit::NanoJoule
        ));
    }
    #[test]
    fn energy_unknown() {
        assert!(EnergyUnit::validate(u("calorie")).is_err());
    }

    // PowerUnit
    #[test]
    fn power_si_abbrev() {
        assert!(matches!(
            PowerUnit::validate(u("mW")).unwrap(),
            PowerUnit::MilliWatt
        ));
        assert!(matches!(
            PowerUnit::validate(u("MW")).unwrap(),
            PowerUnit::MegaWatt
        ));
        assert!(matches!(
            PowerUnit::validate(u("W")).unwrap(),
            PowerUnit::Watt
        ));
    }
    #[test]
    fn power_full_names() {
        assert!(matches!(
            PowerUnit::validate(u("milliwatt")).unwrap(),
            PowerUnit::MilliWatt
        ));
        assert!(matches!(
            PowerUnit::validate(u("megawatts")).unwrap(),
            PowerUnit::MegaWatt
        ));
    }
    #[test]
    fn power_case_insensitive() {
        assert!(matches!(
            PowerUnit::validate(u("MILLIWATT")).unwrap(),
            PowerUnit::MilliWatt
        ));
        assert!(matches!(
            PowerUnit::validate(u("Watt")).unwrap(),
            PowerUnit::Watt
        ));
    }
    #[test]
    fn power_lowercase_mw_is_milliwatt() {
        assert!(matches!(
            PowerUnit::validate(u("mw")).unwrap(),
            PowerUnit::MilliWatt
        ));
    }
    #[test]
    fn power_unknown() {
        assert!(PowerUnit::validate(u("horsepower")).is_err());
    }

    // validate_optional
    #[test]
    fn validate_optional_some() {
        let result: Result<TimeUnit> = validate_optional(Some(u("ms")), TimeUnit::validate);
        assert!(matches!(result.unwrap(), TimeUnit::Milliseconds));
    }
    #[test]
    fn validate_optional_none() {
        let result: Result<TimeUnit> = validate_optional(None, TimeUnit::validate);
        assert!(matches!(result.unwrap(), TimeUnit::Seconds));
    }
    #[test]
    fn validate_optional_error() {
        let result: Result<TimeUnit> = validate_optional(Some(u("parsec")), TimeUnit::validate);
        assert!(result.is_err());
    }
}
