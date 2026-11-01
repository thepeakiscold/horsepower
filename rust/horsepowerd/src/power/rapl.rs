//! Reading RAPL energy counters out of `powercap`.
//!
//! RAPL exposes a monotonic microjoule counter per domain. Differences of that
//! counter give exact watt-hours, so nothing here integrates sampled wattages.
//!
//! Attribution rules, which the GUI depends on:
//!
//! * `psys` is a true platform total and wins when present.
//! * `package-*` is the processor package (cores, cache, uncore and, on client
//!   parts, DRAM).
//! * a nested `dram` counter is a useful *sub-attribution* of the package, and
//!   is only added to the total when there is no `psys` and no package counter.
//! * `core`, `uncore` and `subsystem` are strict subsets of their parent and
//!   are never summed, or energy would be counted twice.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::MICROJOULES_PER_WH;
use crate::power::{Group, SourceError};

/// One `powercap` directory that we can attribute to something.
#[derive(Debug, Clone)]
pub struct Domain {
    pub path: PathBuf,
    pub name: String,
    pub max_energy_uj: u64,
    pub group: Group,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Intel,
    Amd,
}

impl Vendor {
    fn as_str(self) -> &'static str {
        match self {
            Vendor::Intel => "Intel",
            Vendor::Amd => "AMD",
        }
    }
}

/// Parsed shape of a `powercap` directory name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DirName {
    vendor: Vendor,
    mmio: bool,
    depth: usize,
}

/// Parse `intel-rapl:0`, `amd-rapl:1`, `intel-rapl-mmio:0`, `intel-rapl:0:1`.
///
/// Hand-written rather than a regex: the shape is fixed, and a root service
/// that parses directory names from the kernel does not need a regex engine.
fn parse_dir_name(name: &str) -> Option<DirName> {
    let (vendor, rest) = match name.strip_prefix("intel-rapl") {
        Some(rest) => (Vendor::Intel, rest),
        None => (Vendor::Amd, name.strip_prefix("amd-rapl")?),
    };

    let (mmio, rest) = match rest.strip_prefix("-mmio") {
        Some(tail) => (true, tail),
        None => (false, rest),
    };
    let rest = rest.strip_prefix(':')?;

    let mut parts = rest.split(':');
    let first = parts.next()?;
    if first.is_empty() || !first.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let depth = match parts.next() {
        None => 1,
        Some(second) if !second.is_empty() && second.bytes().all(|b| b.is_ascii_digit()) => 2,
        Some(_) => return None,
    };
    if parts.next().is_some() {
        return None;
    }

    Some(DirName {
        vendor,
        mmio,
        depth,
    })
}

/// A set of counters that belong to the same physical interface.
#[derive(Debug)]
pub struct Rapl {
    domains: Vec<Domain>,
    vendor: Vendor,
    mmio: bool,
    last_uj: HashMap<PathBuf, u64>,
}

impl Rapl {
    /// Find the best set of counters available under `powercap`.
    ///
    /// The MSR and MMIO interfaces describe the same hardware, so only one set
    /// is read per tick; otherwise energy would be counted twice. MSR is
    /// preferred because it is present on far more machines.
    pub fn discover(powercap: &Path) -> Option<Self> {
        let entries = fs::read_dir(powercap).ok()?;

        let mut attributed: Vec<(Domain, Vendor, bool)> = Vec::new();
        let mut unknown_top_level: Vec<(Domain, Vendor, bool)> = Vec::new();

        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                continue;
            };
            let Some(parsed) = parse_dir_name(file_name) else {
                continue;
            };

            let path = entry.path();
            let Some(name) = read_trimmed(&path.join("name")) else {
                continue;
            };
            let Some(max_energy_uj) = read_u64(&path.join("max_energy_range_uj")) else {
                log_debug(&format!("ignoring unusable RAPL domain {}", path.display()));
                continue;
            };
            if max_energy_uj == 0 {
                log_debug(&format!(
                    "ignoring zero-range RAPL domain {}",
                    path.display()
                ));
                continue;
            }

            // `dram` may be nested under a package, or sit at the top level on
            // server parts. Everything else is either a top-level total or a
            // subset that must not be summed.
            let (group, known) = match (parsed.depth, name.as_str()) {
                (1, "psys") => (Group::System, true),
                (1, n) if n.starts_with("package") => (Group::Cpu, true),
                (_, "dram") => (Group::Ram, true),
                (1, _) => (Group::Cpu, false),
                _ => continue,
            };

            let domain = Domain {
                path,
                name,
                max_energy_uj,
                group,
            };
            let item = (domain, parsed.vendor, parsed.mmio);
            if known {
                attributed.push(item);
            } else {
                unknown_top_level.push(item);
            }
        }

        // A future kernel may rename the package counter. Reporting *something*
        // is better than hiding the only sensor on the machine, so an
        // unattributed top-level counter is treated as the package.
        if !attributed.iter().any(|(d, _, _)| d.group == Group::Cpu) {
            attributed.extend(unknown_top_level);
        }

        if attributed.is_empty() {
            return None;
        }

        let mmio = !attributed.iter().any(|(_, _, mmio)| !mmio);
        let selected: Vec<_> = attributed
            .into_iter()
            .filter(|(_, _, is_mmio)| *is_mmio == mmio)
            .collect();

        let vendor = if selected.iter().all(|(_, vendor, _)| *vendor == Vendor::Amd) {
            Vendor::Amd
        } else {
            Vendor::Intel
        };

        Some(Self {
            domains: selected.into_iter().map(|(domain, _, _)| domain).collect(),
            vendor,
            mmio,
            last_uj: HashMap::new(),
        })
    }

    pub fn vendor(&self) -> &'static str {
        self.vendor.as_str()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_mmio(&self) -> bool {
        self.mmio
    }

    pub fn scope(&self) -> &'static str {
        if self.domains.iter().any(|d| d.group == Group::System) {
            "system"
        } else {
            "parts"
        }
    }

    pub fn detail(&self) -> String {
        self.domains
            .iter()
            .map(|domain| domain.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Human name such as `Intel RAPL · platform + package + dram`.
    pub fn name(&self) -> String {
        let mut label = format!("{} RAPL", self.vendor());
        if self.mmio {
            label.push_str(" (MMIO)");
        }
        let mut pieces = Vec::new();
        if self.domains.iter().any(|d| d.group == Group::System) {
            pieces.push("platform");
        }
        if self.domains.iter().any(|d| d.group == Group::Cpu) {
            pieces.push("package");
        }
        if self.domains.iter().any(|d| d.group == Group::Ram) {
            pieces.push("dram");
        }
        format!("{label} · {}", pieces.join(" + "))
    }

    fn read_delta(
        last_uj: &mut HashMap<PathBuf, u64>,
        domain: &Domain,
    ) -> Result<u64, SourceError> {
        let energy_path = domain.path.join("energy_uj");
        let current = fs::read_to_string(&energy_path)
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::PermissionDenied {
                    SourceError::Permission(energy_path.display().to_string())
                } else {
                    SourceError::Read(format!("{energy_path:?}: {err}"))
                }
            })?
            .trim()
            .parse::<u64>()
            .map_err(|err| SourceError::Read(format!("{energy_path:?}: {err}")))?;

        let previous = last_uj.insert(domain.path.clone(), current);
        let Some(previous) = previous else {
            // First observation only establishes the baseline.
            return Ok(0);
        };
        if current >= previous {
            Ok(current - previous)
        } else {
            // The counter wrapped. max_energy_range_uj is the largest value
            // before wrapping, hence the + 1.
            Ok((domain.max_energy_uj - previous)
                .saturating_add(current)
                .saturating_add(1))
        }
    }

    /// Read every domain once.
    ///
    /// Returns the delta in microjoules per group. A group with no counter is
    /// absent, which lets the caller distinguish "zero watts" from "not
    /// measurable".
    pub fn read(&mut self) -> Result<Vec<(Group, u64)>, SourceError> {
        let mut totals: HashMap<Group, u64> = HashMap::new();
        // Disjoint field borrows: the domain list is read while the baseline
        // map is written, so no clone per tick is needed.
        let last_uj = &mut self.last_uj;
        for domain in &self.domains {
            let delta = Self::read_delta(last_uj, domain)?;
            *totals.entry(domain.group).or_insert(0) += delta;
        }
        Ok(totals.into_iter().collect())
    }

    /// Reset every baseline so the next read produces no energy.
    pub fn rebaseline(&mut self) {
        self.last_uj.clear();
    }
}

/// Convert a microjoule delta into a reading over `elapsed` seconds.
pub fn microjoules_to_watts(delta_uj: u64, elapsed: f64) -> (f64, f64) {
    if elapsed <= 0.0 {
        return (0.0, 0.0);
    }
    let watts = delta_uj as f64 / 1_000_000.0 / elapsed;
    let energy_wh = delta_uj as f64 / MICROJOULES_PER_WH;
    (watts, energy_wh)
}

pub(crate) fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
}

pub(crate) fn read_u64(path: &Path) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

pub(crate) fn read_i64(path: &Path) -> Option<i64> {
    read_trimmed(path)?.parse().ok()
}

fn log_debug(message: &str) {
    crate::logging::debug(message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    fn write_domain(root: &Path, dir: &str, name: &str, energy: u64, max: u64) -> PathBuf {
        let path = root.join(dir);
        fs::create_dir_all(&path).expect("mkdir");
        fs::write(path.join("name"), name).expect("name");
        fs::write(path.join("energy_uj"), energy.to_string()).expect("energy");
        fs::write(path.join("max_energy_range_uj"), max.to_string()).expect("max");
        path
    }

    #[test]
    fn parses_directory_names() {
        assert_eq!(
            parse_dir_name("intel-rapl:0"),
            Some(DirName {
                vendor: Vendor::Intel,
                mmio: false,
                depth: 1
            })
        );
        assert_eq!(
            parse_dir_name("amd-rapl-mmio:1"),
            Some(DirName {
                vendor: Vendor::Amd,
                mmio: true,
                depth: 1
            })
        );
        assert_eq!(parse_dir_name("intel-rapl:0:2").map(|d| d.depth), Some(2));
        for bad in [
            "intel-rapl",
            "intel-rapl:",
            "intel-rapl:x",
            "intel-rapl:0:1:2",
            "nvidia-rapl:0",
            "intel-rapl:0:",
        ] {
            assert_eq!(parse_dir_name(bad), None, "should reject {bad}");
        }
    }

    #[test]
    fn keeps_psys_packages_and_dram_but_not_subsets() {
        let tmp = tempdir();
        let root = tmp.path().join("powercap");
        write_domain(&root, "intel-rapl:0", "package-0", 0, 1_000_000);
        write_domain(&root, "intel-rapl:0:0", "core", 0, 1_000_000);
        write_domain(&root, "intel-rapl:0:1", "dram", 0, 1_000_000);
        write_domain(&root, "intel-rapl:1", "psys", 0, 1_000_000);

        let rapl = Rapl::discover(&root).expect("rapl");
        let names: Vec<_> = rapl.domains.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"psys"));
        assert!(names.contains(&"package-0"));
        assert!(names.contains(&"dram"));
        assert!(!names.contains(&"core"));
        assert_eq!(rapl.scope(), "system");
    }

    #[test]
    fn prefers_msr_when_both_interfaces_are_listed() {
        let tmp = tempdir();
        let root = tmp.path().join("powercap");
        write_domain(&root, "intel-rapl:0", "package-0", 0, 1_000_000);
        write_domain(&root, "intel-rapl-mmio:0", "package-0", 0, 1_000_000);

        let rapl = Rapl::discover(&root).expect("rapl");
        assert!(!rapl.is_mmio());
        assert_eq!(rapl.domains.len(), 1);
    }

    #[test]
    fn computes_watts_from_counter_delta() {
        let tmp = tempdir();
        let root = tmp.path().join("powercap");
        let domain = write_domain(&root, "intel-rapl:0", "package-0", 1_000_000, 1_000_000_000);
        let mut rapl = Rapl::discover(&root).expect("rapl");

        assert_eq!(rapl.read().expect("first").len(), 1);
        fs::write(domain.join("energy_uj"), "3000000").expect("energy");
        let groups = rapl.read().expect("second");
        let (_, delta) = groups[0];
        let (watts, energy_wh) = microjoules_to_watts(delta, 1.0);
        assert!((watts - 2.0).abs() < 1e-9);
        assert!((energy_wh - 2_000_000.0 / MICROJOULES_PER_WH).abs() < 1e-12);
    }

    #[test]
    fn handles_counter_wrap() {
        let tmp = tempdir();
        let root = tmp.path().join("powercap");
        let domain = write_domain(&root, "intel-rapl:0", "package-0", 950, 1000);
        let mut rapl = Rapl::discover(&root).expect("rapl");
        rapl.read().expect("baseline");
        fs::write(domain.join("energy_uj"), "50").expect("energy");
        let groups = rapl.read().expect("wrapped");
        // 1000 - 950 + 50 + 1 = 101 microjoules.
        assert_eq!(groups[0].1, 101);
    }

    #[test]
    fn promotes_unrecognised_top_level_counter() {
        let tmp = tempdir();
        let root = tmp.path().join("powercap");
        write_domain(&root, "intel-rapl:0", "soc", 0, 1_000_000);
        let rapl = Rapl::discover(&root).expect("rapl");
        assert_eq!(rapl.domains.len(), 1);
        assert_eq!(rapl.domains[0].group, Group::Cpu);
    }

    #[test]
    fn ignores_directories_without_a_range() {
        let tmp = tempdir();
        let root = tmp.path().join("powercap");
        fs::create_dir_all(root.join("intel-rapl:0")).expect("mkdir");
        fs::write(root.join("intel-rapl:0/name"), "package-0").expect("name");
        assert!(Rapl::discover(&root).is_none());
    }

    #[test]
    fn missing_energy_file_is_a_read_error() {
        let tmp = tempdir();
        let root = tmp.path().join("powercap");
        let domain = write_domain(&root, "intel-rapl:0", "package-0", 1, 1_000_000);
        let mut rapl = Rapl::discover(&root).expect("rapl");
        fs::remove_file(domain.join("energy_uj")).expect("unlink");
        assert!(matches!(rapl.read(), Err(SourceError::Read(_))));
    }

    #[test]
    fn group_totals_are_kept_separate() {
        let tmp = tempdir();
        let root = tmp.path().join("powercap");
        let package = write_domain(&root, "intel-rapl:0", "package-0", 0, 1_000_000_000);
        let dram = write_domain(&root, "intel-rapl:0:0", "dram", 0, 1_000_000_000);
        let mut rapl = Rapl::discover(&root).expect("rapl");
        rapl.read().expect("baseline");

        fs::write(package.join("energy_uj"), "10000000").expect("energy");
        fs::write(dram.join("energy_uj"), "2000000").expect("energy");
        let totals: BTreeMap<_, _> = rapl.read().expect("second").into_iter().collect();
        assert_eq!(totals[&Group::Cpu], 10_000_000);
        assert_eq!(totals[&Group::Ram], 2_000_000);
    }
}
