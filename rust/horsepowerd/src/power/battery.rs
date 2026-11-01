//! Battery power, the last resort on machines with no RAPL counters.
//!
//! Laptops generally do not keep the processor counters enabled, but they do
//! expose the battery's own power draw. That number covers the whole machine,
//! so it is reported with `scope = "system"`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::MICROWATTS_PER_WATT;
use crate::power::rapl::{read_i64, read_trimmed};
use crate::power::SourceError;

#[derive(Debug)]
pub struct Battery {
    supplies: Vec<(PathBuf, String)>,
}

impl Battery {
    pub fn discover(power_supply: &Path) -> Option<Self> {
        let entries = fs::read_dir(power_supply).ok()?;
        let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        paths.sort();

        let mut supplies = Vec::new();
        for path in paths {
            if read_trimmed(&path.join("type")).as_deref() != Some("Battery") {
                continue;
            }
            if read_trimmed(&path.join("present")).as_deref() == Some("0") {
                continue;
            }
            // Need either a direct power figure or current and voltage.
            let has_power = path.join("power_now").exists();
            let has_iv = path.join("current_now").exists() && path.join("voltage_now").exists();
            if !has_power && !has_iv {
                continue;
            }
            let name = read_trimmed(&path.join("scope"))
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| name.to_string())
                })
                .unwrap_or_else(|| "battery".to_string());
            supplies.push((path, name));
        }

        if supplies.is_empty() {
            None
        } else {
            Some(Self { supplies })
        }
    }

    pub fn name(&self) -> String {
        if self.supplies.len() == 1 {
            format!("Battery · {}", self.supplies[0].1)
        } else {
            format!("Battery · {} batteries", self.supplies.len())
        }
    }

    pub fn detail(&self) -> String {
        self.supplies
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn supply_watts(path: &Path) -> Result<f64, SourceError> {
        if let Some(power_now) = read_i64(&path.join("power_now")) {
            return Ok(power_now.unsigned_abs() as f64 / MICROWATTS_PER_WATT);
        }
        let current = read_i64(&path.join("current_now")).unwrap_or(0);
        let voltage = read_i64(&path.join("voltage_now")).unwrap_or(0);
        // microamps * microvolts = picowatts
        Ok((current.unsigned_abs() as f64 * voltage.unsigned_abs() as f64) / 1e12)
    }

    pub fn watts(&self) -> Result<f64, SourceError> {
        let mut total = 0.0;
        for (path, _) in &self.supplies {
            total += Self::supply_watts(path)?;
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    fn make_battery(root: &Path, name: &str, power_now: Option<i64>, present: &str) {
        let path = root.join("power_supply").join(name);
        fs::create_dir_all(&path).expect("mkdir");
        fs::write(path.join("type"), "Battery\n").expect("type");
        fs::write(path.join("present"), present).expect("present");
        if let Some(power) = power_now {
            fs::write(path.join("power_now"), power.to_string()).expect("power");
        }
    }

    #[test]
    fn reads_power_now() {
        let tmp = tempdir();
        make_battery(tmp.path(), "BAT0", Some(12_000_000), "1");
        let battery = Battery::discover(&tmp.path().join("power_supply")).expect("battery");
        assert!((battery.watts().expect("watts") - 12.0).abs() < 1e-9);
        assert_eq!(battery.name(), "Battery · BAT0");
    }

    #[test]
    fn falls_back_to_current_times_voltage() {
        let tmp = tempdir();
        let path = tmp.path().join("power_supply/BAT0");
        fs::create_dir_all(&path).expect("mkdir");
        fs::write(path.join("type"), "Battery").expect("type");
        fs::write(path.join("current_now"), "1000000").expect("current");
        fs::write(path.join("voltage_now"), "12000000").expect("voltage");
        let battery = Battery::discover(&tmp.path().join("power_supply")).expect("battery");
        assert!((battery.watts().expect("watts") - 12.0).abs() < 1e-9);
    }

    #[test]
    fn ignores_absent_batteries() {
        let tmp = tempdir();
        make_battery(tmp.path(), "BAT0", Some(12_000_000), "0");
        assert!(Battery::discover(&tmp.path().join("power_supply")).is_none());
    }

    #[test]
    fn ignores_mains_adapters() {
        let tmp = tempdir();
        let path = tmp.path().join("power_supply/AC");
        fs::create_dir_all(&path).expect("mkdir");
        fs::write(path.join("type"), "Mains").expect("type");
        fs::write(path.join("power_now"), "5000000").expect("power");
        assert!(Battery::discover(&tmp.path().join("power_supply")).is_none());
    }
}
