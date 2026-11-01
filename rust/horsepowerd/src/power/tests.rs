//! Breakdown semantics, exercised against a synthetic sysfs tree.
//!
//! These mirror `tests/test_power.py` so the Rust service and the Python
//! reference cannot drift apart in what they report.

use std::fs;

use super::fixtures::{advance, Tree, GIB};
use super::*;

fn watts_of(snapshot: &Snapshot) -> std::collections::HashMap<&'static str, f64> {
    snapshot
        .components
        .iter()
        .map(|component| (component.key, component.watts))
        .collect()
}

#[test]
fn psys_splits_cpu_ram_and_remainder() {
    let tree = Tree::new();
    let psys = tree.domain("intel-rapl:0", "psys", 0, 1_000_000_000);
    let package = tree.domain("intel-rapl:1", "package-0", 0, 1_000_000_000);
    let dram = tree.domain("intel-rapl:1:0", "dram", 0, 1_000_000_000);

    let mut meter = tree.meter();
    meter.sample(1.0).expect("baseline");

    advance(&psys, 100_000_000);
    advance(&package, 55_000_000);
    advance(&dram, 15_000_000);
    let snapshot = meter.sample(1.0).expect("snapshot");

    assert!(snapshot.whole_system);
    assert_eq!(snapshot.scope(), "system");
    assert!((snapshot.watts - 100.0).abs() < 1e-9);

    let watts = watts_of(&snapshot);
    assert!((watts["cpu"] - 55.0).abs() < 1e-9);
    assert!((watts["ram"] - 15.0).abs() < 1e-9);
    assert!((watts["other"] - 30.0).abs() < 1e-9);
}

#[test]
fn discrete_gpu_is_added_on_top_of_psys() {
    // psys does not reach through a discrete card's own VRM, so the card's
    // draw is extra. Subtracting it from the remainder would understate both.
    let tree = Tree::new();
    let psys = tree.domain("intel-rapl:0", "psys", 0, 1_000_000_000);
    let package = tree.domain("intel-rapl:1", "package-0", 0, 1_000_000_000);
    tree.gpu(8 * GIB, 20_000_000);

    let mut meter = tree.meter();
    meter.sample(1.0).expect("baseline");

    advance(&psys, 60_000_000);
    advance(&package, 40_000_000);
    let snapshot = meter.sample(1.0).expect("snapshot");

    assert!((snapshot.watts - 80.0).abs() < 1e-9);
    let watts = watts_of(&snapshot);
    assert!((watts["cpu"] - 40.0).abs() < 1e-9);
    assert!((watts["gpu"] - 20.0).abs() < 1e-9);
    assert!((watts["other"] - 20.0).abs() < 1e-9);

    // The rows must add up to the headline exactly.
    let sum: f64 = watts.values().sum();
    assert!((sum - snapshot.watts).abs() < 1e-9);
}

#[test]
fn parts_scope_invents_no_remainder() {
    let tree = Tree::new();
    let package = tree.domain("intel-rapl:0", "package-0", 0, 1_000_000_000);
    let mut meter = tree.meter();
    meter.sample(1.0).expect("baseline");

    advance(&package, 42_000_000);
    let snapshot = meter.sample(1.0).expect("snapshot");

    assert!(!snapshot.whole_system);
    assert_eq!(snapshot.scope(), "parts");
    assert!((snapshot.watts - 42.0).abs() < 1e-9);
    assert_eq!(snapshot.components.len(), 1);
    assert_eq!(snapshot.components[0].key, "cpu");
}

#[test]
fn unmeasurable_parts_are_absent_not_zero() {
    let tree = Tree::new();
    tree.domain("intel-rapl:0", "package-0", 0, 1_000_000_000);
    let mut meter = tree.meter();
    let snapshot = meter.sample(1.0).expect("snapshot");
    let keys: Vec<_> = snapshot.components.iter().map(|c| c.key).collect();
    assert_eq!(keys, vec!["cpu"]);
}

#[test]
fn integrated_gpu_is_not_counted_twice() {
    let tree = Tree::new();
    let package = tree.domain("intel-rapl:0", "package-0", 0, 1_000_000_000);
    tree.gpu(512 * 1024 * 1024, 9_000_000);
    let mut meter = tree.meter();
    meter.sample(1.0).expect("baseline");
    advance(&package, 30_000_000);
    let snapshot = meter.sample(1.0).expect("snapshot");
    assert!(!watts_of(&snapshot).contains_key("gpu"));
}

#[test]
fn dram_is_added_only_without_psys() {
    let tree = Tree::new();
    let package = tree.domain("intel-rapl:0", "package-0", 0, 1_000_000_000);
    let dram = tree.domain("intel-rapl:0:0", "dram", 0, 1_000_000_000);
    let mut meter = tree.meter();
    meter.sample(1.0).expect("baseline");

    advance(&package, 10_000_000);
    advance(&dram, 2_000_000);
    let snapshot = meter.sample(1.0).expect("snapshot");
    assert!((snapshot.watts - 12.0).abs() < 1e-9);
    let watts = watts_of(&snapshot);
    assert!((watts["cpu"] - 10.0).abs() < 1e-9);
    assert!((watts["ram"] - 2.0).abs() < 1e-9);
}

#[test]
fn dram_is_not_double_counted_under_psys() {
    let tree = Tree::new();
    let psys = tree.domain("intel-rapl:0", "psys", 0, 1_000_000_000);
    let dram = tree.domain("intel-rapl:1", "dram", 0, 1_000_000_000);
    let mut meter = tree.meter();
    meter.sample(1.0).expect("baseline");

    advance(&psys, 40_000_000);
    advance(&dram, 5_000_000);
    let snapshot = meter.sample(1.0).expect("snapshot");
    assert!((snapshot.watts - 40.0).abs() < 1e-9);
}

#[test]
fn energy_tracks_the_counter_not_the_displayed_wattage() {
    let tree = Tree::new();
    let package = tree.domain("intel-rapl:0", "package-0", 0, 1_000_000_000);
    let mut meter = tree.meter();
    meter.sample(1.0).expect("baseline");

    // 3.6 J over two seconds: 1.8 W, and exactly 0.001 Wh.
    advance(&package, 3_600_000);
    let snapshot = meter.sample(2.0).expect("snapshot");
    assert!((snapshot.watts - 1.8).abs() < 1e-9);
    assert!((snapshot.energy_wh - 0.001).abs() < 1e-12);
}

#[test]
fn implausible_delta_is_discarded() {
    let tree = Tree::new();
    let package = tree.domain("intel-rapl:0", "package-0", 0, 1_000_000_000_000);
    let mut meter = tree.meter();
    meter.sample(1.0).expect("baseline");

    // 10 MJ in one second would be 10 MW.
    advance(&package, 10_000_000_000);
    let snapshot = meter.sample(1.0).expect("snapshot");
    assert_eq!(snapshot.watts, 0.0);
    assert_eq!(snapshot.energy_wh, 0.0);
}

#[test]
fn battery_fallback_reports_whole_system() {
    let tree = Tree::new();
    tree.battery(12_000_000);
    let mut meter = tree.meter();
    assert!(meter.available());
    assert_eq!(meter.scope(), "system");

    let snapshot = meter.sample(3600.0).expect("snapshot");
    assert!((snapshot.watts - 12.0).abs() < 1e-9);
    assert!((snapshot.energy_wh - 12.0).abs() < 1e-9);
    assert!(snapshot.whole_system);
    assert_eq!(snapshot.components.len(), 1);
    assert_eq!(snapshot.components[0].key, "system");
}

#[test]
fn no_source_says_so() {
    let tree = Tree::new();
    let meter = tree.meter();
    assert!(!meter.available());
    assert!(meter
        .error()
        .expect("error")
        .contains("No supported power sensor"));
    assert!(meter
        .describe_problem()
        .contains("No supported power sensor"));
}

#[test]
fn unavailable_scope_is_unknown() {
    let tree = Tree::new();
    let meter = tree.meter();
    assert_eq!(meter.scope(), "unknown");
    assert_eq!(meter.source_name(), "No sensor");
}

#[test]
fn source_name_mentions_every_contributor() {
    let tree = Tree::new();
    tree.domain("intel-rapl:0", "psys", 0, 1_000_000_000);
    tree.gpu(8 * GIB, 20_000_000);
    let meter = tree.meter();
    let name = meter.source_name();
    assert!(name.contains("Intel RAPL"), "{name}");
    assert!(name.contains("card1"), "{name}");
}

#[test]
fn storage_power_is_added_on_top_of_psys() {
    let tree = Tree::new();
    let psys = tree.domain("intel-rapl:0", "psys", 0, 1_000_000_000);
    let device = tree.root().join("block/nvme0n1/device");
    let hwmon = device.join("hwmon/hwmon2");
    fs::create_dir_all(&hwmon).expect("mkdir");
    fs::write(hwmon.join("power1_input"), "4500000").expect("power");
    fs::write(device.join("model"), "FastDisk 9000").expect("model");

    let mut meter = tree.meter();
    meter.sample(1.0).expect("baseline");
    advance(&psys, 50_000_000);
    let snapshot = meter.sample(1.0).expect("snapshot");

    let watts = watts_of(&snapshot);
    assert!((watts["storage"] - 4.5).abs() < 1e-9);
    assert!((snapshot.watts - 54.5).abs() < 1e-9);
}

#[test]
fn zero_elapsed_reports_no_energy() {
    let tree = Tree::new();
    tree.domain("intel-rapl:0", "package-0", 0, 1_000_000_000);
    let mut meter = tree.meter();
    meter.sample(1.0).expect("baseline");
    let snapshot = meter.sample(0.0).expect("snapshot");
    assert_eq!(snapshot.watts, 0.0);
    assert_eq!(snapshot.energy_wh, 0.0);
}

#[test]
fn rebaseline_drops_the_next_delta() {
    let tree = Tree::new();
    let package = tree.domain("intel-rapl:0", "package-0", 0, 1_000_000_000);
    let mut meter = tree.meter();
    meter.sample(1.0).expect("baseline");

    advance(&package, 50_000_000);
    meter.rebaseline();
    let snapshot = meter.sample(1.0).expect("snapshot");
    assert_eq!(snapshot.watts, 0.0);
}
