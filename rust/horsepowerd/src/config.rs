//! Values shared by the service, the samplers and the tests.
//!
//! These mirror `src/horsepower/constants.py`. They are duplicated rather than
//! generated because the two halves are released together and a build step
//! that rewrites source is harder to audit than two short constants files.

/// Version string reported over D-Bus and by `--version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Well-known bus name owned by this service.
pub const BUS_NAME: &str = "org.horsepower.Daemon";

/// Object path the interface lives on. The interface itself is named
/// `org.horsepower.Daemon`, declared on the `Daemon` impl in `service.rs` and
/// documented in `data/org.horsepower.Daemon.xml`.
pub const OBJECT_PATH: &str = "/org/horsepower/Daemon";

/// Where the telemetry database lives.
pub const DEFAULT_DB_PATH: &str = "/var/lib/horsepower/power.db";

/// sysfs class root. Overridable for tests and for development.
pub const DEFAULT_SYSFS_ROOT: &str = "/sys/class";

/// Seconds between counter reads.
pub const DEFAULT_SAMPLE_INTERVAL: f64 = 1.0;

/// Minimum interval the sampler accepts. Below this the counter resolution and
/// the scheduling jitter dominate the measurement.
pub const MIN_SAMPLE_INTERVAL: f64 = 0.2;

/// How long per-minute rows are kept. Daily totals are never pruned.
pub const DEFAULT_RETENTION_DAYS: i64 = 90;

/// How often accumulated energy is written to disk. A crash loses at most this
/// much, and the database is not hit on every tick.
pub const FLUSH_INTERVAL: f64 = 20.0;

/// How often an absent sensor is looked for again.
pub const DISCOVERY_RETRY_INTERVAL: f64 = 30.0;

/// Values above this are treated as a counter glitch rather than a reading.
pub const MAX_PLAUSIBLE_WATTS: f64 = 5_000.0;

/// A gap longer than this means the machine slept; the counter is re-baselined
/// instead of booking the whole interval as consumed.
pub const MAX_SAMPLE_GAP: f64 = 120.0;

/// Smoothing factor for the displayed instantaneous values. Fixed because the
/// sampling interval is fixed: deriving it from the measured energy would make
/// the readout *less* calm exactly when the load is changing.
pub const SMOOTHING_ALPHA: f64 = 0.45;

/// Microjoules in one watt-hour.
pub const MICROJOULES_PER_WH: f64 = 3_600_000_000.0;

/// Microwatts in one watt.
pub const MICROWATTS_PER_WATT: f64 = 1_000_000.0;

/// Milliwatts in one watt.
pub const MILLIWATTS_PER_WATT: f64 = 1_000.0;

/// Below this, a GPU is treated as integrated and therefore already counted
/// inside the processor package counter.
pub const MIN_DISCRETE_VRAM: u64 = 1024 * 1024 * 1024;

/// Canonical display order and wording for the breakdown rows.
///
/// A component that the machine cannot measure is absent from the payload, so
/// it is never drawn as a confident `0.0 W`.
pub const COMPONENT_LABELS: [(&str, &str); 6] = [
    ("cpu", "CPU"),
    ("gpu", "GPU"),
    ("ram", "Memory"),
    ("storage", "Storage"),
    ("other", "Rest of system"),
    ("system", "Whole system"),
];

/// Label for a component key, falling back to the key itself.
pub fn label_for(key: &str) -> String {
    COMPONENT_LABELS
        .iter()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, label)| (*label).to_string())
        .unwrap_or_else(|| key.to_string())
}

/// Sort index for a component key; unknown keys sort last.
pub fn component_order(key: &str) -> usize {
    COMPONENT_LABELS
        .iter()
        .position(|(candidate, _)| *candidate == key)
        .unwrap_or(COMPONENT_LABELS.len())
}
