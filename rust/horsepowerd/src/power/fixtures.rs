//! Synthetic sysfs trees shared by the sampler tests.

use std::fs;
use std::path::{Path, PathBuf};

use super::Meter;

pub const GIB: u64 = 1024 * 1024 * 1024;

pub struct Tree {
    dir: tempfile::TempDir,
}

impl Tree {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("temp dir"),
        }
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn domain(&self, dir: &str, name: &str, energy: u64, max: u64) -> PathBuf {
        let path = self.root().join("powercap").join(dir);
        fs::create_dir_all(&path).expect("mkdir");
        fs::write(path.join("name"), name).expect("name");
        fs::write(path.join("energy_uj"), energy.to_string()).expect("energy");
        fs::write(path.join("max_energy_range_uj"), max.to_string()).expect("max");
        path
    }

    pub fn gpu(&self, vram: u64, micro_watts: i64) {
        let device = self.root().join("drm/card1/device");
        let hwmon = device.join("hwmon/hwmon5");
        fs::create_dir_all(&hwmon).expect("mkdir");
        fs::write(device.join("mem_info_vram_total"), vram.to_string()).expect("vram");
        fs::write(hwmon.join("power1_average"), micro_watts.to_string()).expect("power");
    }

    pub fn battery(&self, power_now: i64) {
        let path = self.root().join("power_supply/BAT0");
        fs::create_dir_all(&path).expect("mkdir");
        fs::write(path.join("type"), "Battery").expect("type");
        fs::write(path.join("present"), "1").expect("present");
        fs::write(path.join("power_now"), power_now.to_string()).expect("power");
    }

    /// A meter with NVML disabled, so a developer machine's real card cannot
    /// change the expected numbers.
    pub fn meter(&self) -> Meter {
        Meter::new(self.root().to_path_buf(), false)
    }
}

pub fn advance(path: &Path, microjoules: u64) {
    fs::write(path.join("energy_uj"), microjoules.to_string()).expect("energy");
}
