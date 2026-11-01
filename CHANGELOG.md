# Changelog

All notable changes to HorsePower are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[semantic versioning](https://semver.org/).

## [1.0.0] — 2026-11-01

### Added

- Live power readout from Intel and AMD RAPL energy counters, including the
  platform-wide `psys` domain when the firmware exposes it.
- Per-component breakdown: CPU package, GPU (NVML or hwmon), memory (RAPL
  `dram`), storage where a drive exports a power attribute, and the remainder
  of the platform total. Unmeasurable parts are omitted, never shown as zero.
- Battery fallback for machines without RAPL.
- A background service (`horsepowerd`) that samples once a second and logs to
  SQLite, started at boot and activatable over D-Bus.
- Today, lifetime and cost totals, plus a per-minute chart of the last hour
  that leaves honest gaps while the machine sleeps.
- Tariff and currency preferences, stored per user.
- Debian packaging: desktop entry, AppStream metadata, hicolor and scalable
  icons, manual pages, a hardened systemd unit and a restrictive D-Bus policy.

### Changed

- The metering service is written in Rust (`rust/horsepowerd`). It replaces the
  Python implementation and drops from 36 MB to 2.6 MB of proportional memory
  (PSS) and from 0.10 % to under 0.01 % CPU. The D-Bus interface is unchanged,
  so the GUI required no modification and existing databases keep their
  history.
- NVML is loaded with `dlopen` and only when an NVIDIA display controller is
  present, so machines without one never pay the driver library's memory.
- The package is architecture specific (`amd64`) because the service is a
  compiled binary; SQLite is statically linked, so there is no `libsqlite3-0`
  dependency.

### Notes

- Fans are not individually metered by any mainstream Linux interface; they
  appear only inside "rest of system" when a platform counter exists.
- Displays and USB peripherals are outside `psys` on most desktops. Without a
  platform counter the interface says "measured parts" rather than implying
  whole-system accuracy.
- A discrete GPU's draw is added on top of `psys`, because the platform counter
  does not cover the card's own VRM.

[1.0.0]: https://github.com/thepeakiscold/horsepower/releases/tag/v1.0.0
