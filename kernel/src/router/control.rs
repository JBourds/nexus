//! Typed representation of kernel control files.
//!
//! Replaces raw string splitting/matching in `write_control_file` and
//! `read_control_file` with a single parse step into a concrete enum.

use config::CONTROL_PREFIX;

/// All recognized kernel control files that a protocol process can read/write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlFile {
    // Time
    TimeUs,
    TimeMs,
    TimeS,
    TimeNs,
    // Elapsed
    ElapsedUs,
    ElapsedMs,
    ElapsedS,
    ElapsedNs,
    // Energy
    EnergyLeft,
    EnergyState,
    // Absolute position/orientation
    PosX,
    PosY,
    PosZ,
    PosAz,
    PosEl,
    PosRoll,
    // Relative position delta
    PosDx,
    PosDy,
    PosDz,
    // Motion pattern
    PosMotion,
    // Power flows
    PowerFlows,
}

impl ControlFile {
    /// Parse a filename (e.g. `"ctl.time/us"`) into a `ControlFile` variant.
    /// Returns `None` for unrecognized names.
    pub fn parse(name: &str) -> Option<Self> {
        let suffix = name.strip_prefix(CONTROL_PREFIX)?;
        match suffix {
            "time/us" => Some(Self::TimeUs),
            "time/ms" => Some(Self::TimeMs),
            "time/s" => Some(Self::TimeS),
            "time/ns" => Some(Self::TimeNs),
            "elapsed/us" => Some(Self::ElapsedUs),
            "elapsed/ms" => Some(Self::ElapsedMs),
            "elapsed/s" => Some(Self::ElapsedS),
            "elapsed/ns" => Some(Self::ElapsedNs),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_all_known_files() {
        assert_eq!(ControlFile::parse("ctl.time/us"), Some(ControlFile::TimeUs));
        assert_eq!(ControlFile::parse("ctl.time/ms"), Some(ControlFile::TimeMs));
        assert_eq!(ControlFile::parse("ctl.time/s"), Some(ControlFile::TimeS));
        assert_eq!(ControlFile::parse("ctl.time/ns"), Some(ControlFile::TimeNs));
        assert_eq!(
            ControlFile::parse("ctl.elapsed/us"),
            Some(ControlFile::ElapsedUs)
        );
        assert_eq!(
            ControlFile::parse("ctl.elapsed/ms"),
            Some(ControlFile::ElapsedMs)
        );
        assert_eq!(
            ControlFile::parse("ctl.elapsed/s"),
            Some(ControlFile::ElapsedS)
        );
        assert_eq!(
            ControlFile::parse("ctl.elapsed/ns"),
            Some(ControlFile::ElapsedNs)
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
