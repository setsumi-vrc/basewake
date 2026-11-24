//! Base station identity and power state.

use std::fmt;

/// Power characteristic values of a Valve Lighthouse V2 base station.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    /// 0x00 — fully off (motor and lasers stopped, lowest power). Slower to wake.
    Sleep,
    /// 0x01 — on and tracking.
    On,
    /// 0x02 — motor spun down, quicker to wake.
    Standby,
    /// Any other value: the station reports transitional states (e.g. while booting).
    Transition(u8),
    /// Not read yet, or the last read failed.
    Unknown,
}

impl PowerState {
    pub const BYTE_SLEEP: u8 = 0x00;
    pub const BYTE_ON: u8 = 0x01;
    pub const BYTE_STANDBY: u8 = 0x02;

    pub fn from_byte(b: u8) -> Self {
        match b {
            Self::BYTE_SLEEP => PowerState::Sleep,
            Self::BYTE_ON => PowerState::On,
            Self::BYTE_STANDBY => PowerState::Standby,
            other => PowerState::Transition(other),
        }
    }

    /// True when the station is powered (on, or booting towards on).
    pub fn is_powered(self) -> bool {
        matches!(self, PowerState::On | PowerState::Transition(_))
    }

    /// True when the station is in one of the low-power states.
    pub fn is_off(self) -> bool {
        matches!(self, PowerState::Sleep | PowerState::Standby)
    }

    pub fn label(self) -> String {
        match self {
            PowerState::Sleep => "Sleep".into(),
            PowerState::On => "On".into(),
            PowerState::Standby => "Standby".into(),
            PowerState::Transition(b) => format!("Starting (0x{b:02X})"),
            PowerState::Unknown => "Unknown".into(),
        }
    }
}

impl fmt::Display for PowerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

/// Identifies a base station. Normally its 48-bit Bluetooth address (u64, big-endian byte
/// order, as WinRT and BlueZ give it). macOS never exposes addresses, so there it is the hex
/// part of the advertised name (`LHB-278AF87D`), which is unique per base station; such ids
/// carry the `NAME_ID` marker in the top byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Mac(pub u64);

impl Mac {
    const NAME_ID: u64 = 0xFF << 56;

    pub fn parse(s: &str) -> Option<Mac> {
        let s = s.trim();
        if let Some(hex) = s.strip_prefix(crate::engine::ble::LHB_PREFIX) {
            return Mac::from_name_hex(hex);
        }
        let parts: Vec<&str> = s.split([':', '-']).collect();
        if parts.len() != 6 {
            return None;
        }
        let mut v: u64 = 0;
        for p in parts {
            if p.len() != 2 {
                return None;
            }
            v = (v << 8) | u64::from(u8::from_str_radix(p, 16).ok()?);
        }
        Some(Mac(v))
    }

    /// The id for an advertised name like `LHB-278AF87D` (macOS).
    pub fn from_name(name: &str) -> Option<Mac> {
        Mac::from_name_hex(name.trim().strip_prefix(crate::engine::ble::LHB_PREFIX)?)
    }

    fn from_name_hex(hex: &str) -> Option<Mac> {
        if hex.is_empty() || hex.len() > 8 {
            return None;
        }
        Some(Mac(Mac::NAME_ID | u64::from_str_radix(hex, 16).ok()?))
    }

    /// True for ids made from the name rather than the Bluetooth address.
    pub fn is_name_id(self) -> bool {
        self.0 & Mac::NAME_ID == Mac::NAME_ID
    }
}

impl fmt::Display for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_name_id() {
            return write!(f, "{}{:08X}", crate::engine::ble::LHB_PREFIX, self.0 & 0xFFFF_FFFF);
        }
        let b = self.0.to_be_bytes();
        write!(
            f,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            b[2], b[3], b[4], b[5], b[6], b[7]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_roundtrip() {
        let m = Mac::parse("E1:55:D1:67:66:98").unwrap();
        assert_eq!(m.0, 0xE155D1676698);
        assert_eq!(m.to_string(), "E1:55:D1:67:66:98");
        assert_eq!(Mac::parse("e1-55-d1-67-66-98"), Some(m));
    }

    #[test]
    fn name_ids_roundtrip_and_stay_apart_from_addresses() {
        let m = Mac::from_name("LHB-278AF87D").unwrap();
        assert!(m.is_name_id());
        assert_eq!(m.to_string(), "LHB-278AF87D");
        assert_eq!(Mac::parse("LHB-278AF87D"), Some(m));
        assert!(!Mac::parse("E1:55:D1:67:66:98").unwrap().is_name_id());
        assert!(Mac::from_name("LHB-").is_none());
        assert!(Mac::from_name("LHB-XYZ").is_none());
    }

    #[test]
    fn mac_rejects_garbage() {
        assert!(Mac::parse("E1:55:D1:67:66").is_none());
        assert!(Mac::parse("E1:55:D1:67:66:ZZ").is_none());
        assert!(Mac::parse("E155:D1:67:66:98:00").is_none());
    }

    #[test]
    fn power_bytes() {
        assert_eq!(PowerState::from_byte(0), PowerState::Sleep);
        assert_eq!(PowerState::from_byte(1), PowerState::On);
        assert_eq!(PowerState::from_byte(2), PowerState::Standby);
        assert_eq!(PowerState::from_byte(0x0B), PowerState::Transition(0x0B));
        assert!(PowerState::Transition(9).is_powered());
        assert!(PowerState::Standby.is_off());
        assert!(!PowerState::Unknown.is_off() && !PowerState::Unknown.is_powered());
    }
}
