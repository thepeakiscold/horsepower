//! The sampling state machine and the D-Bus surface.
//!
//! [`Sampler`] is the interesting part and is deliberately free of async and of
//! D-Bus: it takes the time and the elapsed interval, and returns a
//! [`Status`]. That makes the smoothing and rollover rules unit-testable
//! without a bus, a clock, or a running event loop.
//!
//! [`Daemon`] is the thin D-Bus interface over the same shared state.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

use crate::config::{component_order, label_for, FLUSH_INTERVAL, MAX_SAMPLE_GAP, SMOOTHING_ALPHA};
use crate::power::{Meter, Snapshot};
use crate::store::{now_seconds, Store};

/// One component row, in canonical display order.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusRow {
    pub key: String,
    pub label: String,
    pub watts: f64,
}

/// A snapshot of everything the GUI needs, ready to serialise.
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub live_watts: f64,
    pub today_wh: f64,
    pub total_wh: f64,
    pub components: Vec<StatusRow>,
    pub source: String,
    pub scope: String,
    pub detail: String,
    pub available: bool,
    pub message: String,
    pub sample_interval: f64,
    pub retention_days: i64,
    pub first_seen: i64,
    pub updated_at: f64,
}

impl Status {
    /// The `a{sv}` dictionary the D-Bus method and signal carry.
    ///
    /// Key names and value types match the Python service exactly, so the GUI
    /// and any third-party consumer keep working unchanged.
    pub fn to_dict(&self) -> HashMap<String, OwnedValue> {
        let components: Vec<(String, String, f64)> = self
            .components
            .iter()
            .map(|row| (row.key.clone(), row.label.clone(), round(row.watts, 3)))
            .collect();

        let mut dict = HashMap::with_capacity(14);
        insert(&mut dict, "live_watts", round(self.live_watts, 3));
        insert(&mut dict, "today_wh", round(self.today_wh, 4));
        insert(&mut dict, "total_wh", round(self.total_wh, 4));
        insert(
            &mut dict,
            "components",
            components.into_iter().collect::<Vec<_>>(),
        );
        insert(&mut dict, "source", self.source.clone());
        insert(&mut dict, "scope", self.scope.clone());
        insert(&mut dict, "detail", self.detail.clone());
        insert(&mut dict, "available", self.available);
        insert(&mut dict, "message", self.message.clone());
        insert(&mut dict, "sample_interval", self.sample_interval);
        insert(&mut dict, "retention_days", self.retention_days);
        insert(&mut dict, "first_seen", self.first_seen);
        insert(&mut dict, "version", crate::config::VERSION.to_string());
        insert(&mut dict, "updated_at", self.updated_at);
        dict
    }
}

fn insert<T>(dict: &mut HashMap<String, OwnedValue>, key: &str, value: T)
where
    Value<'static>: From<T>,
{
    let owned = OwnedValue::try_from(Value::from(value))
        .expect("value produced from an owned type is always owned");
    dict.insert(key.to_string(), owned);
}

/// Round to `digits` decimal places, matching the Python `round()` usage.
pub fn round(value: f64, digits: i32) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    let factor = 10f64.powi(digits);
    (value * factor).round() / factor
}

/// The sampling state machine.
pub struct Sampler {
    meter: Meter,
    store: Arc<Mutex<Store>>,
    day: String,
    bucket: i64,
    total_wh: f64,
    today_wh: f64,
    pending_wh: f64,
    pending_watt_seconds: f64,
    pending_seconds: f64,
    live_watts: f64,
    live_components: Vec<(String, f64)>,
    scope: &'static str,
    last_sample: Instant,
    next_flush: Instant,
    interval: f64,
    retention_days: i64,
    first_seen: i64,
}

impl Sampler {
    pub fn new(meter: Meter, store: Arc<Mutex<Store>>, interval: f64) -> Self {
        let now = now_seconds();
        let (day, total_wh, today_wh, retention_days, first_seen) = {
            let store = store.lock().expect("store mutex");
            let day = store.local_day(now);
            let (total, today) = store.totals(&day);
            (
                day,
                total,
                today,
                store.retention_days(),
                store.first_seen(),
            )
        };
        let monotonic = Instant::now();
        let meter_scope = meter.scope();
        Self {
            meter,
            store,
            day,
            bucket: (now / 60) * 60,
            total_wh,
            today_wh,
            pending_wh: 0.0,
            pending_watt_seconds: 0.0,
            pending_seconds: 0.0,
            live_watts: 0.0,
            live_components: Vec::new(),
            scope: meter_scope,
            last_sample: monotonic,
            next_flush: monotonic + Duration::from_secs_f64(FLUSH_INTERVAL),
            interval,
            retention_days,
            first_seen,
        }
    }

    /// Take one measurement and return the new status.
    ///
    /// Also reports whether the on-disk log changed, so the caller knows when
    /// to emit `HistoryChanged`.
    pub fn tick(&mut self) -> (Status, bool) {
        let wall = now_seconds();
        let monotonic = Instant::now();
        let mut history_changed = false;

        self.roll_day(wall);

        if !self.meter.available() {
            // Self-throttled; asking every tick is cheap and keeps a sensor
            // that appears later (dock, driver load) picked up promptly.
            self.meter.refresh(false);
        }

        // Monotonic time keeps the wattage honest across clock adjustments;
        // wall time is only used for bucketing and day boundaries.
        let elapsed = monotonic
            .saturating_duration_since(self.last_sample)
            .as_secs_f64();
        self.last_sample = monotonic;

        let snapshot = if elapsed > MAX_SAMPLE_GAP {
            // The machine slept or the process was frozen. Counters may have
            // wrapped or stopped, so re-baseline instead of booking a spike.
            crate::logging::info(&format!("Ignoring a {elapsed:.0}s gap in sampling"));
            self.meter.rebaseline();
            None
        } else {
            self.meter.sample(elapsed)
        };

        if let Some(snapshot) = snapshot {
            if elapsed > 0.0 {
                let bucket = (wall / 60) * 60;
                if bucket != self.bucket {
                    history_changed |= self.flush(wall as f64);
                    self.bucket = bucket;
                }

                self.pending_wh += snapshot.energy_wh;
                self.pending_watt_seconds += snapshot.watts * elapsed;
                self.pending_seconds += elapsed;
                self.smooth(&snapshot);
                self.scope = snapshot.scope();

                if monotonic >= self.next_flush {
                    history_changed |= self.flush(wall as f64);
                }
            }
        }

        (self.status(wall as f64), history_changed)
    }

    /// Blend the new row values into the displayed ones.
    ///
    /// Rows are smoothed first and the headline is then derived as their sum,
    /// so the five numbers on screen can never disagree during a ramp.
    fn smooth(&mut self, snapshot: &Snapshot) {
        let mut seen: Vec<String> = Vec::with_capacity(snapshot.components.len());
        for component in &snapshot.components {
            let key = component.key.to_string();
            seen.push(key.clone());
            let value = match self.live_components.iter().find(|(k, _)| *k == key) {
                None => component.watts,
                Some((_, previous)) => blend(*previous, component.watts),
            };
            match self.live_components.iter_mut().find(|(k, _)| *k == key) {
                Some(entry) => entry.1 = value,
                None => self.live_components.push((key, value)),
            }
        }
        // A row that is no longer measured must disappear, not freeze.
        self.live_components.retain(|(key, _)| seen.contains(key));

        self.live_watts = if self.live_components.is_empty() {
            blend(self.live_watts, snapshot.watts)
        } else {
            self.live_components.iter().map(|(_, watts)| watts).sum()
        };
    }

    /// Write accumulated energy to disk. Returns whether anything was written.
    fn flush(&mut self, wall: f64) -> bool {
        if self.pending_seconds <= 0.0 {
            return false;
        }
        let now = wall as i64;
        let day_now = {
            let Ok(mut store) = self.store.lock() else {
                crate::logging::warn("store is poisoned; keeping energy in memory");
                return false;
            };
            if let Err(err) = store.record(
                self.bucket,
                &self.day,
                self.pending_wh,
                self.pending_watt_seconds,
                self.pending_seconds,
            ) {
                // Keep the data pending and retry; a full disk or a locked
                // database must not silently lose measured energy.
                crate::logging::warn(&format!("could not record samples: {err}"));
                return false;
            }
            store.prune(now);
            store.local_day(now)
        };

        self.total_wh += self.pending_wh;
        if self.day == day_now {
            self.today_wh += self.pending_wh;
        }
        crate::logging::debug(&format!(
            "Flushed {:.4} Wh (total {:.3} Wh, today {:.3} Wh)",
            self.pending_wh, self.total_wh, self.today_wh
        ));

        self.pending_wh = 0.0;
        self.pending_watt_seconds = 0.0;
        self.pending_seconds = 0.0;
        self.next_flush = Instant::now() + Duration::from_secs_f64(FLUSH_INTERVAL);
        true
    }

    /// Close out the current day and load the next day's running total.
    fn roll_day(&mut self, wall: i64) {
        let day = match self.store.lock() {
            Ok(store) => store.local_day(wall),
            Err(_) => return,
        };
        if day == self.day {
            return;
        }
        self.flush(wall as f64);
        self.day = day.clone();
        self.today_wh = match self.store.lock() {
            Ok(store) => store.today_total(&day),
            Err(_) => 0.0,
        };
        crate::logging::info(&format!("New day: {day}"));
    }

    /// The current status without taking a measurement first.
    pub fn status_now(&self) -> Status {
        self.status(now_seconds() as f64)
    }

    /// Flush and checkpoint; used on shutdown.
    pub fn shutdown(&mut self) {
        self.flush(now_seconds() as f64);
        if let Ok(store) = self.store.lock() {
            store.checkpoint();
        }
    }

    fn status(&self, wall: f64) -> Status {
        let mut components: Vec<StatusRow> = self
            .live_components
            .iter()
            .map(|(key, watts)| StatusRow {
                key: key.clone(),
                label: label_for(key),
                watts: *watts,
            })
            .collect();
        components.sort_by_key(|row| component_order(&row.key));

        Status {
            live_watts: self.live_watts.max(0.0),
            // Include the interval that has not been flushed yet, otherwise the
            // totals would look frozen for up to FLUSH_INTERVAL seconds.
            today_wh: self.today_wh + self.pending_wh,
            total_wh: self.total_wh + self.pending_wh,
            components,
            source: self.meter.source_name(),
            scope: self.scope.to_string(),
            detail: self.meter.detail(),
            available: self.meter.available(),
            message: self.meter.message(),
            sample_interval: self.interval,
            retention_days: self.retention_days,
            first_seen: self.first_seen,
            updated_at: wall,
        }
    }
}

/// Exponential blend with a floor, so an idle machine settles at a true zero
/// instead of carrying a permanent 0.04 W of rounding dust.
fn blend(previous: f64, current: f64) -> f64 {
    if previous <= 0.0 {
        // First observation after startup should show the real figure at once
        // rather than creeping up from zero.
        return current.max(0.0);
    }
    let value = previous + SMOOTHING_ALPHA * (current - previous);
    if value < 0.05 {
        0.0
    } else {
        value
    }
}

/// The D-Bus interface.
pub struct Daemon {
    status: Arc<Mutex<Status>>,
    store: Arc<Mutex<Store>>,
}

impl Daemon {
    pub fn new(status: Arc<Mutex<Status>>, store: Arc<Mutex<Store>>) -> Self {
        Self { status, store }
    }
}

#[zbus::interface(name = "org.horsepower.Daemon")]
impl Daemon {
    /// Current reading, totals and the per-component breakdown.
    async fn get_status(&self) -> HashMap<String, OwnedValue> {
        match self.status.lock() {
            Ok(status) => status.to_dict(),
            Err(_) => Status::default().to_dict(),
        }
    }

    /// `(timestamp, watts, watt_hours)` per minute for the last `minutes`.
    async fn get_history(&self, minutes: u32) -> Vec<(i64, f64, f64)> {
        let now = now_seconds();
        let Ok(store) = self.store.lock() else {
            return Vec::new();
        };
        store
            .history(minutes, now)
            .into_iter()
            .map(|point| (point.bucket, point.watts, point.wh))
            .collect()
    }

    #[zbus(signal)]
    pub async fn status_changed(
        emitter: &SignalEmitter<'_>,
        status: HashMap<String, OwnedValue>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn history_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::Component;

    #[test]
    fn rounds_for_display() {
        assert_eq!(round(1.23456, 3), 1.235);
        assert_eq!(round(0.00049, 4), 0.0005);
        assert_eq!(round(0.00004, 4), 0.0);
        // A non-finite value must never reach the wire as `nan`: the GUI would
        // render it as a dash and lose the reading.
        assert_eq!(round(f64::NAN, 3), 0.0);
        assert_eq!(round(f64::INFINITY, 3), 0.0);
    }

    #[test]
    fn dict_has_every_documented_key() {
        let status = Status {
            components: vec![StatusRow {
                key: "cpu".into(),
                label: "CPU".into(),
                watts: 12.3456,
            }],
            ..Status::default()
        };
        let dict = status.to_dict();
        for key in [
            "live_watts",
            "today_wh",
            "total_wh",
            "components",
            "source",
            "scope",
            "detail",
            "available",
            "message",
            "sample_interval",
            "retention_days",
            "first_seen",
            "version",
            "updated_at",
        ] {
            assert!(dict.contains_key(key), "missing {key}");
        }
        // Components must serialise as a(ssd), which is what the GUI unpacks.
        use zbus::zvariant::{Type, Value};
        assert_eq!(<Vec<(String, String, f64)> as Type>::SIGNATURE, "a(ssd)");
        assert_eq!(
            dict["components"].value_signature(),
            &Value::from(Vec::<(String, String, f64)>::new())
                .value_signature()
                .clone()
        );
    }

    #[test]
    fn smoothing_snaps_first_sample_then_blends() {
        assert_eq!(blend(0.0, 42.0), 42.0);
        assert!((blend(42.0, 0.0) - 42.0 * (1.0 - SMOOTHING_ALPHA)).abs() < 1e-12);
        // An idle machine settles at a true zero, not 0.04 W of dust.
        assert_eq!(blend(0.04, 0.0), 0.0);
    }

    fn sampler_with(trees: &crate::power::fixtures::Tree) -> (Arc<Mutex<Store>>, Sampler) {
        let store = Arc::new(Mutex::new(
            Store::open(&trees.root().join("power.db"), 90).expect("store"),
        ));
        let meter = Meter::new(trees.root().to_path_buf(), false);
        let sampler = Sampler::new(meter, Arc::clone(&store), 1.0);
        (store, sampler)
    }

    #[test]
    fn components_are_ordered_and_labelled() {
        let tree = crate::power::fixtures::Tree::new();
        tree.battery(10_000_000);
        let (_store, mut sampler) = sampler_with(&tree);
        let (status, _) = sampler.tick();
        assert_eq!(status.components.len(), 1);
        assert_eq!(status.components[0].key, "system");
        assert_eq!(status.components[0].label, "Whole system");
    }

    #[test]
    fn headline_equals_the_sum_of_the_rows() {
        let tree = crate::power::fixtures::Tree::new();
        tree.battery(10_000_000);
        let (_store, mut sampler) = sampler_with(&tree);
        sampler.smooth(&Snapshot {
            watts: 60.0,
            energy_wh: 1.0,
            components: vec![
                Component {
                    key: "cpu",
                    watts: 40.0,
                },
                Component {
                    key: "ram",
                    watts: 20.0,
                },
            ],
            whole_system: false,
        });
        let sum: f64 = sampler.live_components.iter().map(|(_, w)| w).sum();
        assert!((sampler.live_watts - sum).abs() < 1e-12);
    }

    #[test]
    fn stale_components_disappear() {
        let tree = crate::power::fixtures::Tree::new();
        tree.battery(10_000_000);
        let (_store, mut sampler) = sampler_with(&tree);
        sampler.smooth(&Snapshot {
            watts: 60.0,
            energy_wh: 1.0,
            components: vec![
                Component {
                    key: "cpu",
                    watts: 40.0,
                },
                Component {
                    key: "ram",
                    watts: 20.0,
                },
            ],
            whole_system: false,
        });
        assert_eq!(sampler.live_components.len(), 2);

        sampler.smooth(&Snapshot {
            watts: 30.0,
            energy_wh: 0.5,
            components: vec![Component {
                key: "cpu",
                watts: 30.0,
            }],
            whole_system: false,
        });
        assert_eq!(sampler.live_components.len(), 1);
        assert_eq!(sampler.live_components[0].0, "cpu");
        assert!(sampler.live_watts > 30.0, "smoothing should lag upwards");
    }
}
