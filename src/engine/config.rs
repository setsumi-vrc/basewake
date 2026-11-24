//! `basewake.toml`: user settings and the list of known base stations.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::station::{Mac, PowerState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OffMode {
    #[default]
    Sleep,
    Standby,
}

impl OffMode {
    pub fn byte(self) -> u8 {
        match self {
            OffMode::Sleep => PowerState::BYTE_SLEEP,
            OffMode::Standby => PowerState::BYTE_STANDBY,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationConfig {
    pub mac: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

impl StationConfig {
    pub fn mac(&self) -> Option<Mac> {
        Mac::parse(&self.mac)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub off_mode: OffMode,
    /// Scan for new base stations every time SteamVR starts.
    #[serde(default = "yes")]
    pub scan_on_start: bool,
    /// Newly found base stations are enabled (managed) right away.
    #[serde(default = "yes")]
    pub auto_enable_new: bool,
    /// Quit the app after SteamVR closes (otherwise it stays in the tray).
    #[serde(default = "yes")]
    pub close_with_steamvr: bool,
    #[serde(default, rename = "station")]
    pub stations: Vec<StationConfig>,
}

fn yes() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config {
            off_mode: OffMode::Sleep,
            scan_on_start: true,
            auto_enable_new: true,
            close_with_steamvr: true,
            stations: Vec::new(),
        }
    }
}

impl Config {
    /// `%APPDATA%\Basewake`
    pub fn dir() -> PathBuf {
        directories::BaseDirs::new()
            .map(|d| d.config_dir().join("Basewake"))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn path() -> PathBuf {
        Self::dir().join("basewake.toml")
    }

    /// Loads the config; a missing file gives the defaults. A broken file is an error
    /// (so a typo never silently wipes the station list).
    pub fn load() -> Result<Config> {
        Self::migrate_legacy();
        Self::load_from(&Self::path())
    }

    /// The app used to be called "Lighthouse Manager": carry its settings and station list over once.
    fn migrate_legacy() {
        let new = Self::path();
        if new.exists() {
            return;
        }
        let Some(old) = directories::BaseDirs::new()
            .map(|d| d.config_dir().join("LighthouseManager").join("lighthouse.toml"))
            .filter(|p| p.exists())
        else {
            return;
        };
        let _ = fs::create_dir_all(Self::dir());
        match fs::copy(&old, &new) {
            Ok(_) => tracing::info!("settings carried over from {}", old.display()),
            Err(e) => tracing::warn!("could not carry over {}: {e}", old.display()),
        }
    }

    pub fn load_from(path: &Path) -> Result<Config> {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("invalid {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }

    /// Writes to a temp file and renames it over the old one, so a crash never leaves half a file.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let text = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).with_context(|| format!("cannot write {}", tmp.display()))?;
        fs::rename(&tmp, path).with_context(|| format!("cannot replace {}", path.display()))?;
        Ok(())
    }

    pub fn find(&self, mac: Mac) -> Option<&StationConfig> {
        self.stations.iter().find(|s| s.mac() == Some(mac))
    }

    pub fn find_mut(&mut self, mac: Mac) -> Option<&mut StationConfig> {
        self.stations.iter_mut().find(|s| s.mac() == Some(mac))
    }

    /// Adds a newly discovered station. Returns true if it was not known before.
    pub fn add_discovered(&mut self, mac: Mac, name: &str) -> bool {
        if let Some(existing) = self.find_mut(mac) {
            if existing.name.is_empty() && !name.is_empty() {
                existing.name = name.to_string();
            }
            return false;
        }
        self.stations.push(StationConfig {
            mac: mac.to_string(),
            name: name.to_string(),
            enabled: self.auto_enable_new,
        });
        true
    }

    pub fn enabled_macs(&self) -> Vec<Mac> {
        self.stations.iter().filter(|s| s.enabled).filter_map(|s| s.mac()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_missing() {
        let c = Config::load_from(Path::new("Z:/definitely/not/here.toml")).unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(c.off_mode, OffMode::Sleep);
    }

    #[test]
    fn roundtrip_and_partial_file() {
        let dir = std::env::temp_dir().join(format!("lh-test-{}", std::process::id()));
        let path = dir.join("basewake.toml");
        let mut c = Config { off_mode: OffMode::Standby, ..Config::default() };
        assert!(c.add_discovered(Mac::parse("E1:55:D1:67:66:98").unwrap(), "LHB-1"));
        assert!(!c.add_discovered(Mac::parse("E1:55:D1:67:66:98").unwrap(), "LHB-1"));
        c.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path).unwrap(), c);

        // Missing keys fall back to defaults.
        std::fs::write(&path, "[[station]]\nmac = \"E5:07:28:58:FB:83\"\n").unwrap();
        let p = Config::load_from(&path).unwrap();
        assert!(p.scan_on_start && p.stations[0].enabled);
        assert_eq!(p.off_mode, OffMode::Sleep);

        std::fs::write(&path, "off_mode = 3").unwrap();
        assert!(Config::load_from(&path).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn new_station_respects_auto_enable() {
        let mut c = Config { auto_enable_new: false, ..Config::default() };
        c.add_discovered(Mac(1), "LHB-X");
        assert!(!c.stations[0].enabled);
        assert!(c.enabled_macs().is_empty());
    }
}
