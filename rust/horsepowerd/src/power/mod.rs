//! Power measurement.
//!
//! [`Meter`] owns every source this machine offers and turns them into one
//! [`Snapshot`] per tick: a total, the energy consumed during the interval, and
//! the rows that explain the total.
//!
//! Two rules keep the breakdown honest:
//!
//! 1. A discrete GPU and a disk sit behind their own power delivery, so RAPL's
//!    `psys` does **not** include them. Their draw is added *on top of* the
//!    platform figure. Subtracting it from the remainder instead would
//!    understate both the row and the headline.
//! 2. A remainder is only computed against a real platform total. When only
//!    package counters exist the scope is `parts`, and no "rest of system"
//!    figure is invented.

pub mod battery;
pub mod hwmon;
pub mod nvml;
pub mod rapl;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::config::{DISCOVERY_RETRY_INTERVAL, MAX_PLAUSIBLE_WATTS};

/// Which counter group a RAPL domain belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    System,
    Cpu,
    Ram,
}

/// A failure to read hardware, with enough detail to tell the user why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    Permission(String),
    Read(String),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Permission(path) => {
                write!(formatter, "Permission denied reading {path}")
            }
            SourceError::Read(message) => write!(formatter, "Cannot read {message}"),
        }
    }
}

impl std::error::Error for SourceError {}

/// One measured part of the machine.
#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    pub key: &'static str,
    pub watts: f64,
}

/// Everything one sampling tick produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub watts: f64,
    pub energy_wh: f64,
    pub components: Vec<Component>,
    /// `true` when `watts` is a real whole-platform figure, `false` when it is
    /// only the sum of the parts we can see.
    pub whole_system: bool,
}

impl Snapshot {
    pub fn scope(&self) -> &'static str {
        if self.whole_system {
            "system"
        } else {
            "parts"
        }
    }
}

/// Whichever graphics backend this machine offers.
enum Gpu {
    Nvml(nvml::Nvml),
    Hwmon(hwmon::HwmonGpu),
}

/// Is there an NVIDIA display controller on the PCI bus?
///
/// NVML is only worth its resident cost on machines that can answer. Loading
/// the driver library on a machine without a card would add roughly 20 MB of
/// mostly-shared pages to a root daemon for nothing, so the presence of an
/// NVIDIA VGA/3D controller gates the `dlopen`.
///
/// The PCI directory is found by walking up from `sysfs_root`, because the
/// default root is `/sys/class` while PCI lives at `/sys/bus/pci`. If no PCI
/// tree is reachable (a test fixture, a container with a trimmed `/sys`) the
/// answer is `false`: skipping NVML loses a GPU row, while loading it blindly
/// costs memory on every machine that will never have one.
fn has_nvidia_display(sysfs_root: &Path) -> bool {
    let mut directory = sysfs_root.to_path_buf();
    for _ in 0..4 {
        let devices = directory.join("bus/pci/devices");
        if let Ok(entries) = std::fs::read_dir(&devices) {
            for entry in entries.flatten() {
                let path = entry.path();
                let vendor = rapl::read_trimmed(&path.join("vendor"));
                // 0x030000 is a VGA controller, 0x030200 a 3D controller.
                let class = rapl::read_trimmed(&path.join("class"));
                let is_nvidia = vendor.as_deref() == Some("0x10de");
                let is_display = class
                    .as_deref()
                    .map(|value| value.starts_with("0x03"))
                    .unwrap_or(false);
                if is_nvidia && is_display {
                    return true;
                }
            }
            return false;
        }
        if !directory.pop() {
            break;
        }
    }
    false
}

impl Gpu {
    fn discover(sysfs_root: &Path, enable_nvml: bool) -> Option<Self> {
        if enable_nvml && has_nvidia_display(sysfs_root) {
            if let Some(session) = nvml::Nvml::open() {
                return Some(Gpu::Nvml(session));
            }
        }
        hwmon::HwmonGpu::discover(&sysfs_root.join("drm")).map(Gpu::Hwmon)
    }

    fn name(&self) -> String {
        match self {
            Gpu::Nvml(session) => {
                let names = session.names();
                match names.len() {
                    0 => "Graphics".to_string(),
                    1 => names[0].clone(),
                    _ => format!("{} +{}", names[0], names.len() - 1),
                }
            }
            Gpu::Hwmon(gpu) => {
                let names = gpu.names();
                match names.len() {
                    0 => "Graphics".to_string(),
                    1 => names[0].to_string(),
                    _ => format!("{} +{}", names[0], names.len() - 1),
                }
            }
        }
    }

    fn watts(&self) -> Result<f64, SourceError> {
        match self {
            Gpu::Nvml(session) => session.watts(),
            Gpu::Hwmon(gpu) => gpu.watts(),
        }
    }
}

/// Owns the available sources and produces attributable snapshots.
pub struct Meter {
    sysfs_root: PathBuf,
    enable_nvml: bool,
    rapl: Option<rapl::Rapl>,
    battery: Option<battery::Battery>,
    gpu: Option<Gpu>,
    storage: Option<hwmon::HwmonStorage>,
    error: Option<String>,
    permission_denied: bool,
    last_probe: Option<Instant>,
}

impl Meter {
    pub fn new(sysfs_root: impl Into<PathBuf>, enable_nvml: bool) -> Self {
        let mut meter = Self {
            sysfs_root: sysfs_root.into(),
            enable_nvml,
            rapl: None,
            battery: None,
            gpu: None,
            storage: None,
            error: None,
            permission_denied: false,
            last_probe: None,
        };
        meter.refresh(true);
        meter
    }

    pub fn available(&self) -> bool {
        self.rapl.is_some() || self.battery.is_some()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Why measurement is unavailable, in words a user can act on.
    pub fn describe_problem(&self) -> String {
        if self.available() {
            return String::new();
        }
        if self.permission_denied {
            return "The energy counter is root-only on this system. The HorsePower \
                    background service must run to read it."
                .to_string();
        }
        self.error
            .clone()
            .unwrap_or_else(|| "Power measurement is unavailable.".to_string())
    }

    /// What the GUI shows under the reading.
    pub fn message(&self) -> String {
        match self.error.clone() {
            Some(error) if !error.is_empty() => error,
            _ => self.describe_problem(),
        }
    }

    pub fn source_name(&self) -> String {
        let mut parts = Vec::new();
        if let Some(rapl) = &self.rapl {
            parts.push(rapl.name());
        } else if let Some(battery) = &self.battery {
            parts.push(battery.name());
        }
        if let Some(gpu) = &self.gpu {
            parts.push(gpu.name());
        }
        if parts.is_empty() {
            "No sensor".to_string()
        } else {
            parts.join(" · ")
        }
    }

    pub fn scope(&self) -> &'static str {
        if let Some(rapl) = &self.rapl {
            rapl.scope()
        } else if self.battery.is_some() {
            "system"
        } else {
            "unknown"
        }
    }

    pub fn detail(&self) -> String {
        if let Some(rapl) = &self.rapl {
            let mut detail = rapl.detail();
            if let Some(gpu) = &self.gpu {
                detail.push_str(&format!("; gpu {}", gpu.name()));
            }
            if let Some(storage) = &self.storage {
                detail.push_str(&format!("; disk {}", storage.names().join(", ")));
            }
            return detail;
        }
        if let Some(battery) = &self.battery {
            return battery.detail();
        }
        self.error.clone().unwrap_or_default()
    }

    /// Look for hardware again, at most every few seconds.
    pub fn refresh(&mut self, force: bool) {
        if !force {
            if let Some(last) = self.last_probe {
                if last.elapsed() < Duration::from_secs_f64(DISCOVERY_RETRY_INTERVAL.min(5.0)) {
                    return;
                }
            }
        }
        self.last_probe = Some(Instant::now());

        let powercap = self.sysfs_root.join("powercap");
        let rapl = rapl::Rapl::discover(&powercap);
        let battery = if rapl.is_some() {
            None
        } else {
            battery::Battery::discover(&self.sysfs_root.join("power_supply"))
        };

        // Opening NVML is comparatively expensive, so it is done once and kept.
        if self.gpu.is_none() {
            self.gpu = Gpu::discover(&self.sysfs_root, self.enable_nvml);
        }
        if self.storage.is_none() {
            self.storage = hwmon::HwmonStorage::discover(&self.sysfs_root);
        }

        self.rapl = rapl;
        self.battery = battery;
        self.permission_denied = false;

        if self.rapl.is_none() && self.battery.is_none() {
            self.error = Some(
                "No supported power sensor was found. HorsePower needs Intel or AMD RAPL, \
                 or a battery with a power_now attribute."
                    .to_string(),
            );
            return;
        }

        // Probe once so a root-only counter is reported clearly at startup
        // rather than after the first tick.
        let probe = match &mut self.rapl {
            Some(rapl) => rapl.read().map(|_| ()),
            None => match &self.battery {
                Some(battery) => battery.watts().map(|_| ()),
                None => Ok(()),
            },
        };

        match probe {
            Ok(()) => self.error = None,
            Err(err) => {
                self.error = Some(err.to_string());
                if matches!(err, SourceError::Permission(_)) {
                    self.permission_denied = true;
                    self.rapl = None;
                    self.battery = None;
                }
                crate::logging::warn(&err.to_string());
            }
        }
    }

    /// Reset the counters so the next sample books no energy.
    pub fn rebaseline(&mut self) {
        if let Some(rapl) = &mut self.rapl {
            rapl.rebaseline();
        }
    }

    /// Produce one snapshot, or `None` when nothing is measurable.
    pub fn sample(&mut self, elapsed: f64) -> Option<Snapshot> {
        if self.rapl.is_none() && self.battery.is_none() {
            self.refresh(false);
            if self.rapl.is_none() && self.battery.is_none() {
                return None;
            }
        }

        let gpu_watts = match &self.gpu {
            Some(gpu) => gpu.watts().unwrap_or_else(|err| {
                crate::logging::debug(&format!("GPU read failed: {err}"));
                0.0
            }),
            None => 0.0,
        };
        let storage_watts = match &self.storage {
            Some(storage) => storage.watts().unwrap_or_else(|err| {
                crate::logging::debug(&format!("storage read failed: {err}"));
                0.0
            }),
            None => 0.0,
        };
        let extra_watts = gpu_watts + storage_watts;

        if let Some(rapl) = &mut self.rapl {
            let groups = match rapl.read() {
                Ok(groups) => groups,
                Err(err) => {
                    self.error = Some(err.to_string());
                    crate::logging::warn(&err.to_string());
                    if matches!(err, SourceError::Permission(_)) {
                        self.permission_denied = true;
                        self.rapl = None;
                    }
                    return None;
                }
            };
            self.error = None;

            let raw: std::collections::HashMap<Group, u64> = groups.into_iter().collect();
            let cpu = raw.get(&Group::Cpu).map(|&delta| convert(delta, elapsed));
            let ram = raw.get(&Group::Ram).map(|&delta| convert(delta, elapsed));
            let system = raw
                .get(&Group::System)
                .map(|&delta| convert(delta, elapsed));

            let mut components = Vec::new();
            if let Some((watts, _)) = cpu {
                components.push(Component { key: "cpu", watts });
            }
            if let Some((watts, _)) = ram {
                components.push(Component { key: "ram", watts });
            }

            if let Some((system_watts, system_wh)) = system {
                let attributed: f64 = components.iter().map(|c| c.watts).sum();
                let remainder = system_watts - attributed;
                if remainder > 0.05 {
                    components.push(Component {
                        key: "other",
                        watts: remainder,
                    });
                }
                if gpu_watts > 0.0 {
                    components.push(Component {
                        key: "gpu",
                        watts: gpu_watts,
                    });
                }
                if storage_watts > 0.0 {
                    components.push(Component {
                        key: "storage",
                        watts: storage_watts,
                    });
                }
                return Some(Snapshot {
                    watts: system_watts + extra_watts,
                    energy_wh: system_wh + extra_watts * elapsed.max(0.0) / 3600.0,
                    components,
                    whole_system: true,
                });
            }

            if gpu_watts > 0.0 {
                components.push(Component {
                    key: "gpu",
                    watts: gpu_watts,
                });
            }
            if storage_watts > 0.0 {
                components.push(Component {
                    key: "storage",
                    watts: storage_watts,
                });
            }
            let energy_wh = cpu.map(|(_, wh)| wh).unwrap_or(0.0)
                + ram.map(|(_, wh)| wh).unwrap_or(0.0)
                + extra_watts * elapsed.max(0.0) / 3600.0;
            let watts: f64 = components.iter().map(|c| c.watts).sum();
            return Some(Snapshot {
                watts,
                energy_wh,
                components,
                whole_system: false,
            });
        }

        let battery = self.battery.as_ref()?;
        match battery.watts() {
            Ok(watts) => {
                let watts = if watts > MAX_PLAUSIBLE_WATTS {
                    0.0
                } else {
                    watts
                };
                self.error = None;
                Some(Snapshot {
                    watts,
                    energy_wh: watts * elapsed.max(0.0) / 3600.0,
                    components: vec![Component {
                        key: "system",
                        watts,
                    }],
                    whole_system: true,
                })
            }
            Err(err) => {
                self.error = Some(err.to_string());
                crate::logging::warn(&err.to_string());
                None
            }
        }
    }
}

/// Turn a microjoule delta into `(watts, watt_hours)`.
///
/// A delta that implies an absurd wattage is a counter glitch or an unhandled
/// reset, and is dropped rather than poisoning the totals.
fn convert(delta_uj: u64, elapsed: f64) -> (f64, f64) {
    if elapsed <= 0.0 {
        return (0.0, 0.0);
    }
    let (watts, energy_wh) = rapl::microjoules_to_watts(delta_uj, elapsed);
    if watts > MAX_PLAUSIBLE_WATTS {
        crate::logging::warn(&format!(
            "discarding implausible delta ({delta_uj} uJ in {elapsed:.2}s)"
        ));
        return (0.0, 0.0);
    }
    (watts.max(0.0), energy_wh.max(0.0))
}

#[cfg(test)]
pub mod fixtures;
#[cfg(test)]
mod tests;
