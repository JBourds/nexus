use config::CONTROL_PREFIX;
use config::ast::TimeUnit;

use crate::channel::ChannelMode;
use crate::fs::FsEntryKind;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControlFile {
    Time(TimeUnit),
    SleepRelative(TimeUnit),
    SleepAbsolute(TimeUnit),
    Elapsed(TimeUnit),
    EnergyLeft,
    EnergyState,
    PowerFlows,
    PosX,
    PosY,
    PosZ,
    PosAz,
    PosEl,
    PosRoll,
    PosDx,
    PosDy,
    PosDz,
    PosMotion,
}

impl ControlFile {
    /// Parse a filename (e.g. `"ctl.time/us"`) into a `ControlFile` variant.
    /// Returns `None` for unrecognized names.
    pub fn parse(name: &str) -> Option<Self> {
        let suffix = name.strip_prefix(CONTROL_PREFIX)?;
        match suffix {
            "sleep.relative/ns" => Some(Self::SleepRelative(TimeUnit::Nanoseconds)),
            "sleep.relative/us" => Some(Self::SleepRelative(TimeUnit::Microseconds)),
            "sleep.relative/ms" => Some(Self::SleepRelative(TimeUnit::Milliseconds)),
            "sleep.relative/s" => Some(Self::SleepRelative(TimeUnit::Seconds)),
            "sleep.absolute/ns" => Some(Self::SleepAbsolute(TimeUnit::Nanoseconds)),
            "sleep.absolute/us" => Some(Self::SleepAbsolute(TimeUnit::Microseconds)),
            "sleep.absolute/ms" => Some(Self::SleepAbsolute(TimeUnit::Milliseconds)),
            "sleep.absolute/s" => Some(Self::SleepAbsolute(TimeUnit::Seconds)),
            "time/ns" => Some(Self::Time(TimeUnit::Nanoseconds)),
            "time/us" => Some(Self::Time(TimeUnit::Microseconds)),
            "time/ms" => Some(Self::Time(TimeUnit::Milliseconds)),
            "time/s" => Some(Self::Time(TimeUnit::Seconds)),
            "elapsed/ns" => Some(Self::Elapsed(TimeUnit::Nanoseconds)),
            "elapsed/us" => Some(Self::Elapsed(TimeUnit::Microseconds)),
            "elapsed/ms" => Some(Self::Elapsed(TimeUnit::Milliseconds)),
            "elapsed/s" => Some(Self::Elapsed(TimeUnit::Seconds)),
            "energy_left" => Some(Self::EnergyLeft),
            "energy_state" => Some(Self::EnergyState),
            "pos/x" => Some(Self::PosX),
            "pos/y" => Some(Self::PosY),
            "pos/z" => Some(Self::PosZ),
            "pos/az" => Some(Self::PosAz),
            "pos/el" => Some(Self::PosEl),
            "pos/roll" => Some(Self::PosRoll),
            "pos/dx" => Some(Self::PosDx),
            "pos/dy" => Some(Self::PosDy),
            "pos/dz" => Some(Self::PosDz),
            "pos/motion" => Some(Self::PosMotion),
            "power_flows" => Some(Self::PowerFlows),
            _ => None,
        }
    }
}

/// Returns all control file paths for the resolver to inject into channel names.
pub fn control_files() -> Vec<String> {
    let mut files = Vec::new();
    for (name, _, _) in CONTROL_FILES.iter() {
        files.push(name.to_string());
    }
    for (name, _, _) in TIME_SUBFILES.iter() {
        files.push(format!("ctl.time/{name}"));
    }
    for (name, _, _) in ELAPSED_SUBFILES.iter() {
        files.push(format!("ctl.elapsed/{name}"));
    }
    for (name, _, _) in SLEEP_RELATIVE_SUBFILES.iter() {
        files.push(format!("ctl.sleep.relative/{name}"));
    }
    for (name, _, _) in SLEEP_ABSOLUTE_SUBFILES.iter() {
        files.push(format!("ctl.sleep.absolute/{name}"));
    }
    for (name, _, _) in POS_SUBFILES.iter() {
        files.push(format!("ctl.pos/{name}"));
    }
    files
}

/// Flat control files that remain at the root level (not in subdirectories).
pub(crate) const CONTROL_FILES: [(&str, ChannelMode, FsEntryKind); 3] = [
    (
        "ctl.energy_left",
        ChannelMode::ReadOnly,
        FsEntryKind::ControlFile(ControlFile::EnergyLeft),
    ),
    (
        "ctl.energy_state",
        ChannelMode::ReadWrite,
        FsEntryKind::ControlFile(ControlFile::EnergyState),
    ),
    (
        "ctl.power_flows",
        ChannelMode::ReadWrite,
        FsEntryKind::ControlFile(ControlFile::PowerFlows),
    ),
];

/// Sub-files under the `ctl.time/` directory.
pub(crate) const TIME_SUBFILES: [(&str, ChannelMode, FsEntryKind); 4] = [
    (
        "s",
        ChannelMode::ReadWrite,
        FsEntryKind::ControlFile(ControlFile::Time(TimeUnit::Seconds)),
    ),
    (
        "ms",
        ChannelMode::ReadWrite,
        FsEntryKind::ControlFile(ControlFile::Time(TimeUnit::Milliseconds)),
    ),
    (
        "us",
        ChannelMode::ReadWrite,
        FsEntryKind::ControlFile(ControlFile::Time(TimeUnit::Microseconds)),
    ),
    (
        "ns",
        ChannelMode::ReadWrite,
        FsEntryKind::ControlFile(ControlFile::Time(TimeUnit::Nanoseconds)),
    ),
];

/// Sub-files under the `ctl.elapsed/` directory.
pub(crate) const ELAPSED_SUBFILES: [(&str, ChannelMode, FsEntryKind); 4] = [
    (
        "s",
        ChannelMode::ReadOnly,
        FsEntryKind::ControlFile(ControlFile::Elapsed(TimeUnit::Seconds)),
    ),
    (
        "ms",
        ChannelMode::ReadOnly,
        FsEntryKind::ControlFile(ControlFile::Elapsed(TimeUnit::Milliseconds)),
    ),
    (
        "us",
        ChannelMode::ReadOnly,
        FsEntryKind::ControlFile(ControlFile::Elapsed(TimeUnit::Microseconds)),
    ),
    (
        "ns",
        ChannelMode::ReadOnly,
        FsEntryKind::ControlFile(ControlFile::Elapsed(TimeUnit::Nanoseconds)),
    ),
];

/// Sub-files under the `ctl.sleep.relative` directory.
pub(crate) const SLEEP_RELATIVE_SUBFILES: [(&str, ChannelMode, FsEntryKind); 4] = [
    (
        "s",
        ChannelMode::WriteOnly,
        FsEntryKind::ControlFile(ControlFile::SleepRelative(TimeUnit::Seconds)),
    ),
    (
        "ms",
        ChannelMode::WriteOnly,
        FsEntryKind::ControlFile(ControlFile::SleepRelative(TimeUnit::Milliseconds)),
    ),
    (
        "us",
        ChannelMode::WriteOnly,
        FsEntryKind::ControlFile(ControlFile::SleepRelative(TimeUnit::Microseconds)),
    ),
    (
        "ns",
        ChannelMode::WriteOnly,
        FsEntryKind::ControlFile(ControlFile::SleepRelative(TimeUnit::Nanoseconds)),
    ),
];

/// Sub-files under the `ctl.sleep.absolute` directory.
pub(crate) const SLEEP_ABSOLUTE_SUBFILES: [(&str, ChannelMode, FsEntryKind); 4] = [
    (
        "s",
        ChannelMode::WriteOnly,
        FsEntryKind::ControlFile(ControlFile::SleepAbsolute(TimeUnit::Seconds)),
    ),
    (
        "ms",
        ChannelMode::WriteOnly,
        FsEntryKind::ControlFile(ControlFile::SleepAbsolute(TimeUnit::Milliseconds)),
    ),
    (
        "us",
        ChannelMode::WriteOnly,
        FsEntryKind::ControlFile(ControlFile::SleepAbsolute(TimeUnit::Microseconds)),
    ),
    (
        "ns",
        ChannelMode::WriteOnly,
        FsEntryKind::ControlFile(ControlFile::SleepAbsolute(TimeUnit::Nanoseconds)),
    ),
];

/// Sub-files under the `ctl.pos/` directory.
pub(crate) const POS_SUBFILES: [(&str, ChannelMode, FsEntryKind); 10] = [
    (
        "x",
        ChannelMode::ReadWrite,
        FsEntryKind::ControlFile(ControlFile::PosX),
    ),
    (
        "y",
        ChannelMode::ReadWrite,
        FsEntryKind::ControlFile(ControlFile::PosY),
    ),
    (
        "z",
        ChannelMode::ReadWrite,
        FsEntryKind::ControlFile(ControlFile::PosZ),
    ),
    (
        "az",
        ChannelMode::ReadWrite,
        FsEntryKind::ControlFile(ControlFile::PosAz),
    ),
    (
        "el",
        ChannelMode::ReadWrite,
        FsEntryKind::ControlFile(ControlFile::PosEl),
    ),
    (
        "roll",
        ChannelMode::ReadWrite,
        FsEntryKind::ControlFile(ControlFile::PosRoll),
    ),
    (
        "dx",
        ChannelMode::WriteOnly,
        FsEntryKind::ControlFile(ControlFile::PosDx),
    ),
    (
        "dy",
        ChannelMode::WriteOnly,
        FsEntryKind::ControlFile(ControlFile::PosDy),
    ),
    (
        "dz",
        ChannelMode::WriteOnly,
        FsEntryKind::ControlFile(ControlFile::PosDz),
    ),
    (
        "motion",
        ChannelMode::ReadWrite,
        FsEntryKind::ControlFile(ControlFile::PosMotion),
    ),
];

/// Sub-files under each channel directory (e.g., `lora/`).
pub(crate) const CHANNEL_SUBFILES: [(&str, ChannelMode); 3] = [
    ("channel", ChannelMode::ReadWrite), // mode overridden per channel
    ("rssi", ChannelMode::ReadOnly),
    ("snr", ChannelMode::ReadOnly),
];

#[cfg(test)]
mod tests {
    use config::ast::TimeUnit;

    use super::*;

    #[test]
    fn parse_all_known_files() {
        assert_eq!(
            ControlFile::parse("ctl.time/us"),
            Some(ControlFile::Time(TimeUnit::Microseconds))
        );
        assert_eq!(
            ControlFile::parse("ctl.time/ms"),
            Some(ControlFile::Time(TimeUnit::Milliseconds))
        );
        assert_eq!(
            ControlFile::parse("ctl.time/s"),
            Some(ControlFile::Time(TimeUnit::Seconds))
        );
        assert_eq!(
            ControlFile::parse("ctl.time/ns"),
            Some(ControlFile::Time(TimeUnit::Nanoseconds))
        );
        assert_eq!(
            ControlFile::parse("ctl.elapsed/us"),
            Some(ControlFile::Elapsed(TimeUnit::Microseconds))
        );
        assert_eq!(
            ControlFile::parse("ctl.elapsed/ms"),
            Some(ControlFile::Elapsed(TimeUnit::Milliseconds))
        );
        assert_eq!(
            ControlFile::parse("ctl.elapsed/s"),
            Some(ControlFile::Elapsed(TimeUnit::Seconds))
        );
        assert_eq!(
            ControlFile::parse("ctl.elapsed/ns"),
            Some(ControlFile::Elapsed(TimeUnit::Nanoseconds))
        );
        assert_eq!(
            ControlFile::parse("ctl.energy_left"),
            Some(ControlFile::EnergyLeft)
        );
        assert_eq!(
            ControlFile::parse("ctl.energy_state"),
            Some(ControlFile::EnergyState)
        );
        assert_eq!(ControlFile::parse("ctl.pos/x"), Some(ControlFile::PosX));
        assert_eq!(ControlFile::parse("ctl.pos/y"), Some(ControlFile::PosY));
        assert_eq!(ControlFile::parse("ctl.pos/z"), Some(ControlFile::PosZ));
        assert_eq!(ControlFile::parse("ctl.pos/az"), Some(ControlFile::PosAz));
        assert_eq!(ControlFile::parse("ctl.pos/el"), Some(ControlFile::PosEl));
        assert_eq!(
            ControlFile::parse("ctl.pos/roll"),
            Some(ControlFile::PosRoll)
        );
        assert_eq!(ControlFile::parse("ctl.pos/dx"), Some(ControlFile::PosDx));
        assert_eq!(ControlFile::parse("ctl.pos/dy"), Some(ControlFile::PosDy));
        assert_eq!(ControlFile::parse("ctl.pos/dz"), Some(ControlFile::PosDz));
        assert_eq!(
            ControlFile::parse("ctl.pos/motion"),
            Some(ControlFile::PosMotion)
        );
        assert_eq!(
            ControlFile::parse("ctl.power_flows"),
            Some(ControlFile::PowerFlows)
        );
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert_eq!(ControlFile::parse("ctl.unknown"), None);
        assert_eq!(ControlFile::parse("not_a_ctl_file"), None);
        assert_eq!(ControlFile::parse(""), None);
    }

    #[test]
    fn parse_old_dot_format_returns_none() {
        // Old format should no longer parse
        assert_eq!(ControlFile::parse("ctl.time.us"), None);
        assert_eq!(ControlFile::parse("ctl.elapsed.ms"), None);
    }

    #[test]
    fn parse_channel_names_return_none() {
        assert_eq!(ControlFile::parse("my_channel"), None);
        assert_eq!(ControlFile::parse("radio0"), None);
    }
}
