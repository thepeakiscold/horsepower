//! Minimal logging to stderr.
//!
//! systemd/journald captures stderr and adds its own timestamps, so this
//! deliberately stays a line-oriented format identical to the Python service:
//!
//! ```text
//! horsepowerd: INFO: Connected to the bus as org.horsepower.Daemon
//! ```
//!
//! No logging framework is linked: the whole service emits about six distinct
//! messages, and a root daemon is easier to trust when its dependency tree is
//! short enough to read.

use std::sync::atomic::{AtomicU8, Ordering};

const WARN: u8 = 1;
const INFO: u8 = 2;
const DEBUG: u8 = 3;

static LEVEL: AtomicU8 = AtomicU8::new(INFO);

/// Raise the level to include debug output.
pub fn set_verbose(verbose: bool) {
    LEVEL.store(if verbose { DEBUG } else { INFO }, Ordering::Relaxed);
}

fn level() -> u8 {
    LEVEL.load(Ordering::Relaxed)
}

fn emit(name: &str, message: &str) {
    eprintln!("horsepowerd: {name}: {message}");
}

pub fn error(message: &str) {
    emit("ERROR", message);
}

pub fn warn(message: &str) {
    if level() >= WARN {
        emit("WARNING", message);
    }
}

pub fn info(message: &str) {
    if level() >= INFO {
        emit("INFO", message);
    }
}

pub fn debug(message: &str) {
    if level() >= DEBUG {
        emit("DEBUG", message);
    }
}
