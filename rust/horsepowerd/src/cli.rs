//! Command line parsing.
//!
//! Hand written: the service has seven flags and no subcommands, so a parser
//! crate would add a dozen transitive dependencies to a root daemon for no
//! gain. The parser is total (every input has a defined result) and tested.

use std::path::PathBuf;

use crate::config::{
    DEFAULT_DB_PATH, DEFAULT_RETENTION_DAYS, DEFAULT_SAMPLE_INTERVAL, DEFAULT_SYSFS_ROOT,
    MIN_SAMPLE_INTERVAL, VERSION,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusKind {
    System,
    Session,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub db_path: PathBuf,
    pub sysfs_root: PathBuf,
    pub interval: f64,
    pub retention_days: i64,
    pub bus: BusKind,
    pub verbose: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from(DEFAULT_DB_PATH),
            sysfs_root: PathBuf::from(DEFAULT_SYSFS_ROOT),
            interval: DEFAULT_SAMPLE_INTERVAL,
            retention_days: DEFAULT_RETENTION_DAYS,
            bus: BusKind::System,
            verbose: false,
        }
    }
}

/// What the process should do after parsing.
#[derive(Debug, PartialEq)]
pub enum Action {
    Run(Config),
    Help,
    Version,
}

pub const USAGE: &str = "\
usage: horsepowerd [-h] [--foreground] [--db DB] [--sysfs-root SYSFS_ROOT]
                   [--interval INTERVAL] [--retention-days RETENTION_DAYS]
                   [--bus {system,session}] [--verbose] [--version]

HorsePower power metering service.

options:
  -h, --help            show this help message and exit
  --foreground          accepted for compatibility; always runs in the foreground
  --db DB               telemetry database (default: /var/lib/horsepower/power.db)
  --sysfs-root SYSFS_ROOT
                        sysfs class root, useful for testing (default: /sys/class)
  --interval INTERVAL   sampling interval in seconds (default: 1)
  --retention-days RETENTION_DAYS
                        how many days of minute data to keep (default: 90)
  --bus {system,session}
                        D-Bus to use (default: system)
  --verbose, -v         enable debug logging
  --version             show the program version and exit
";

pub fn parse<I, S>(arguments: I) -> Result<Action, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut config = Config::default();
    let mut arguments = arguments.into_iter().map(Into::into).peekable();

    while let Some(argument) = arguments.next() {
        // Support both `--flag value` and `--flag=value`.
        let (flag, inline) = match argument.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                (flag.to_string(), Some(value.to_string()))
            }
            _ => (argument.clone(), None),
        };

        let mut take_value = |name: &str| -> Result<String, String> {
            if let Some(value) = inline.clone() {
                return Ok(value);
            }
            arguments
                .next()
                .ok_or_else(|| format!("argument {name}: expected one value"))
        };

        match flag.as_str() {
            "-h" | "--help" => return Ok(Action::Help),
            "--version" => return Ok(Action::Version),
            "--foreground" => {
                // Kept so existing unit files and scripts keep working.
            }
            "--verbose" | "-v" => config.verbose = true,
            "--db" => {
                let value = take_value("--db")?;
                if value.trim().is_empty() {
                    return Err("argument --db: path must not be empty".to_string());
                }
                config.db_path = PathBuf::from(value);
            }
            "--sysfs-root" => {
                let value = take_value("--sysfs-root")?;
                if value.trim().is_empty() {
                    return Err("argument --sysfs-root: path must not be empty".to_string());
                }
                config.sysfs_root = PathBuf::from(value);
            }
            "--interval" => {
                let value = take_value("--interval")?;
                let parsed = value
                    .parse::<f64>()
                    .map_err(|_| format!("argument --interval: invalid float value: '{value}'"))?;
                if !parsed.is_finite() || parsed <= 0.0 {
                    return Err("argument --interval: must be a positive number".to_string());
                }
                config.interval = parsed.max(MIN_SAMPLE_INTERVAL);
            }
            "--retention-days" => {
                let value = take_value("--retention-days")?;
                let parsed = value.parse::<i64>().map_err(|_| {
                    format!("argument --retention-days: invalid int value: '{value}'")
                })?;
                if parsed < 1 {
                    return Err("argument --retention-days: must be at least 1".to_string());
                }
                config.retention_days = parsed;
            }
            "--bus" => {
                let value = take_value("--bus")?;
                config.bus = match value.as_str() {
                    "system" => BusKind::System,
                    "session" => BusKind::Session,
                    other => {
                        return Err(format!(
                            "argument --bus: invalid choice: '{other}' (choose from 'system', 'session')"
                        ))
                    }
                };
            }
            other => return Err(format!("unrecognized arguments: {other}")),
        }
    }

    Ok(Action::Run(config))
}

/// `--version` output, matching the Python service.
pub fn version_line() -> String {
    VERSION.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Config {
        match parse(args.iter().copied()).expect("parse") {
            Action::Run(config) => config,
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn defaults_match_the_documented_values() {
        let config = run(&[]);
        assert_eq!(
            config.db_path,
            PathBuf::from("/var/lib/horsepower/power.db")
        );
        assert_eq!(config.sysfs_root, PathBuf::from("/sys/class"));
        assert_eq!(config.interval, 1.0);
        assert_eq!(config.retention_days, 90);
        assert_eq!(config.bus, BusKind::System);
        assert!(!config.verbose);
    }

    #[test]
    fn parses_every_flag() {
        let config = run(&[
            "--db",
            "/tmp/x.db",
            "--sysfs-root",
            "/tmp/sys",
            "--interval",
            "0.5",
            "--retention-days",
            "7",
            "--bus",
            "session",
            "--verbose",
            "--foreground",
        ]);
        assert_eq!(config.db_path, PathBuf::from("/tmp/x.db"));
        assert_eq!(config.sysfs_root, PathBuf::from("/tmp/sys"));
        assert_eq!(config.interval, 0.5);
        assert_eq!(config.retention_days, 7);
        assert_eq!(config.bus, BusKind::Session);
        assert!(config.verbose);
    }

    #[test]
    fn accepts_equals_form() {
        let config = run(&["--db=/tmp/y.db", "--interval=2"]);
        assert_eq!(config.db_path, PathBuf::from("/tmp/y.db"));
        assert_eq!(config.interval, 2.0);
    }

    #[test]
    fn clamps_the_interval_to_something_measurable() {
        let config = run(&["--interval", "0.001"]);
        assert_eq!(config.interval, MIN_SAMPLE_INTERVAL);
    }

    #[test]
    fn rejects_bad_input() {
        for args in [
            vec!["--interval", "nope"],
            vec!["--interval", "-1"],
            vec!["--retention-days", "0"],
            vec!["--bus", "garbage"],
            vec!["--nonsense"],
            vec!["--db"],
        ] {
            assert!(
                parse(args.iter().copied()).is_err(),
                "should reject {args:?}"
            );
        }
    }

    #[test]
    fn help_and_version_short_circuit() {
        assert_eq!(parse(["--help"]).expect("parse"), Action::Help);
        assert_eq!(parse(["-h"]).expect("parse"), Action::Help);
        assert_eq!(parse(["--version"]).expect("parse"), Action::Version);
        // Even with other arguments later.
        assert_eq!(
            parse(["--version", "--db", "/tmp/x"]).expect("parse"),
            Action::Version
        );
    }
}
