//! Graphics and storage power read from the kernel's hwmon interface.
//!
//! This is the portable path: it works for AMD and Intel discrete cards, for
//! Nouveau, and for the handful of drives that export a power sensor. Most
//! devices export temperature only, in which case they are simply absent from
//! the breakdown rather than reported as zero.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{MICROWATTS_PER_WATT, MIN_DISCRETE_VRAM};
use crate::power::rapl::{read_i64, read_trimmed};
use crate::power::SourceError;

/// A readable power attribute plus the divisor that turns it into watts.
#[derive(Debug, Clone)]
pub struct PowerAttribute {
    pub path: PathBuf,
    pub scale: f64,
}

impl PowerAttribute {
    pub fn watts(&self) -> Result<f64, SourceError> {
        let raw = read_i64(&self.path)
            .ok_or_else(|| SourceError::Read(format!("cannot read {}", self.path.display())))?;
        Ok((raw.unsigned_abs() as f64) / self.scale)
    }
}

/// Find `power1_average` or `power1_input` under `device/hwmon/hwmon*`.
///
/// `power1_average` is preferred: it is what the driver already smooths. Both
/// are reported in microwatts.
pub fn find_power_attribute(device: &Path) -> Option<PowerAttribute> {
    let hwmon = device.join("hwmon");
    let entries = fs::read_dir(&hwmon).ok()?;
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();

    for file in ["power1_average", "power1_input"] {
        for dir in &dirs {
            let candidate = dir.join(file);
            if read_i64(&candidate).is_some() {
                return Some(PowerAttribute {
                    path: candidate,
                    scale: MICROWATTS_PER_WATT,
                });
            }
        }
    }
    None
}

/// A discrete GPU reachable through hwmon.
#[derive(Debug)]
pub struct HwmonGpu {
    devices: Vec<(PowerAttribute, String)>,
}

impl HwmonGpu {
    pub fn discover(drm: &Path) -> Option<Self> {
        let entries = fs::read_dir(drm).ok()?;
        let mut cards: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| {
                        name.starts_with("card") && name[4..].chars().all(|c| c.is_ascii_digit())
                    })
                    .unwrap_or(false)
            })
            .collect();
        cards.sort();

        let mut devices = Vec::new();
        let mut seen: Vec<PathBuf> = Vec::new();
        for card in cards {
            let device = card.join("device");
            if !device.is_dir() {
                continue;
            }
            let key = fs::canonicalize(&device).unwrap_or_else(|_| device.clone());
            if seen.contains(&key) {
                continue;
            }
            // An integrated GPU is already inside the processor package
            // counter; adding it again would overstate the machine.
            if let Some(vram) = read_i64(&device.join("mem_info_vram_total")) {
                if (vram as u64) < MIN_DISCRETE_VRAM {
                    continue;
                }
            }
            let Some(attribute) = find_power_attribute(&device) else {
                continue;
            };
            seen.push(key);
            let name = read_trimmed(&device.join("product_name"))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| {
                    card.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("GPU")
                        .to_string()
                });
            devices.push((attribute, name));
        }

        if devices.is_empty() {
            None
        } else {
            Some(Self { devices })
        }
    }

    pub fn names(&self) -> Vec<&str> {
        self.devices.iter().map(|(_, name)| name.as_str()).collect()
    }

    pub fn watts(&self) -> Result<f64, SourceError> {
        let mut total = 0.0;
        for (attribute, _) in &self.devices {
            total += attribute.watts()?;
        }
        Ok(total)
    }
}

/// Disks that expose a real power sensor. Rare, but free to check.
#[derive(Debug)]
pub struct HwmonStorage {
    devices: Vec<(PowerAttribute, String)>,
}

impl HwmonStorage {
    pub fn discover(sysfs_root: &Path) -> Option<Self> {
        let mut devices = Vec::new();
        let mut seen: Vec<PathBuf> = Vec::new();

        for class in ["nvme", "block"] {
            let root = sysfs_root.join(class);
            let Ok(entries) = fs::read_dir(&root) else {
                continue;
            };
            let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
            paths.sort();

            for entry in paths {
                let device = entry.join("device");
                if !device.is_dir() {
                    continue;
                }
                let key = fs::canonicalize(&device).unwrap_or_else(|_| device.clone());
                if seen.contains(&key) {
                    continue;
                }
                let Some(attribute) = find_power_attribute(&device) else {
                    continue;
                };
                seen.push(key);
                let name = read_trimmed(&device.join("model"))
                    .filter(|value| !value.is_empty())
                    .or_else(|| read_trimmed(&entry.join("device/model")))
                    .unwrap_or_else(|| {
                        entry
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("disk")
                            .to_string()
                    });
                devices.push((attribute, name));
            }
        }

        if devices.is_empty() {
            None
        } else {
            Some(Self { devices })
        }
    }

    pub fn names(&self) -> Vec<&str> {
        self.devices.iter().map(|(_, name)| name.as_str()).collect()
    }

    pub fn watts(&self) -> Result<f64, SourceError> {
        let mut total = 0.0;
        for (attribute, _) in &self.devices {
            total += attribute.watts()?;
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

    fn make_card(root: &Path, card: &str, vram: Option<u64>, micro_watts: Option<i64>) -> PathBuf {
        let device = root.join("drm").join(card).join("device");
        let hwmon = device.join("hwmon").join("hwmon5");
        fs::create_dir_all(&hwmon).expect("mkdir");
        if let Some(vram) = vram {
            fs::write(device.join("mem_info_vram_total"), vram.to_string()).expect("vram");
        }
        if let Some(watts) = micro_watts {
            fs::write(hwmon.join("power1_average"), watts.to_string()).expect("power");
        }
        hwmon
    }

    #[test]
    fn measures_a_discrete_card() {
        let tmp = tempdir();
        make_card(
            tmp.path(),
            "card1",
            Some(8 * 1024 * 1024 * 1024),
            Some(23_000_000),
        );
        let gpu = HwmonGpu::discover(&tmp.path().join("drm")).expect("gpu");
        assert!((gpu.watts().expect("watts") - 23.0).abs() < 1e-9);
    }

    #[test]
    fn skips_integrated_graphics() {
        let tmp = tempdir();
        make_card(
            tmp.path(),
            "card0",
            Some(512 * 1024 * 1024),
            Some(9_000_000),
        );
        assert!(HwmonGpu::discover(&tmp.path().join("drm")).is_none());
    }

    #[test]
    fn skips_cards_without_a_power_sensor() {
        let tmp = tempdir();
        let hwmon = make_card(tmp.path(), "card2", Some(8 * 1024 * 1024 * 1024), None);
        fs::write(hwmon.join("temp1_input"), "42000").expect("temp");
        assert!(HwmonGpu::discover(&tmp.path().join("drm")).is_none());
    }

    #[test]
    fn sums_two_cards() {
        let tmp = tempdir();
        make_card(
            tmp.path(),
            "card1",
            Some(8 * 1024 * 1024 * 1024),
            Some(20_000_000),
        );
        make_card(
            tmp.path(),
            "card2",
            Some(8 * 1024 * 1024 * 1024),
            Some(5_000_000),
        );
        let gpu = HwmonGpu::discover(&tmp.path().join("drm")).expect("gpu");
        assert!((gpu.watts().expect("watts") - 25.0).abs() < 1e-9);
        assert_eq!(gpu.names().len(), 2);
    }

    #[test]
    fn finds_storage_power() {
        let tmp = tempdir();
        let device = tmp.path().join("block/sda/device");
        let hwmon = device.join("hwmon/hwmon3");
        fs::create_dir_all(&hwmon).expect("mkdir");
        fs::write(hwmon.join("power1_input"), "4500000").expect("power");
        fs::write(device.join("model"), "FastDisk 9000\n").expect("model");

        let storage = HwmonStorage::discover(tmp.path()).expect("storage");
        assert!((storage.watts().expect("watts") - 4.5).abs() < 1e-9);
        assert_eq!(storage.names(), vec!["FastDisk 9000"]);
    }

    #[test]
    fn storage_without_power_is_absent() {
        let tmp = tempdir();
        let hwmon = tmp.path().join("block/sda/device/hwmon/hwmon3");
        fs::create_dir_all(&hwmon).expect("mkdir");
        fs::write(hwmon.join("temp1_input"), "31000").expect("temp");
        assert!(HwmonStorage::discover(tmp.path()).is_none());
    }
}
