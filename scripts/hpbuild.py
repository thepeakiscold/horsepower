"""Shared packaging logic for HorsePower.

``stage_tree`` lays out an installed file tree that both the standalone
``build-deb.py`` script and a debhelper ``debian/rules`` file can use, so the
file list is written down exactly once.
"""

from __future__ import annotations

import gzip
import hashlib
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PACKAGE = "horsepower"
VERSION = "1.0.0"

#: The daemon is a compiled Rust binary, so the package is architecture
#: specific even though the GUI half is plain Python.
RUST_DIR = ROOT / "rust" / "horsepowerd"
DAEMON_BINARY = RUST_DIR / "target" / "release" / "horsepowerd"
MAINTAINER = "HorsePower contributors <maintainer@horsepower.invalid>"
HOMEPAGE = "https://github.com/thepeakiscold/horsepower"

#: libc6 >= 2.34 provides dlopen in libc itself, which is the only thing the
#: Rust binary needs beyond the kernel. The binary is linked against libc and
#: libgcc_s only; SQLite is compiled in.
DEPENDS = ", ".join(
    [
        "libc6 (>= 2.34)",
        "python3 (>= 3.10)",
        "python3-gi (>= 3.42)",
        "python3-gi-cairo",
        "python3-cairo",
        "gir1.2-gtk-4.0 (>= 4.10)",
        "gir1.2-adw-1 (>= 1.5)",
        "init-system-helpers (>= 1.52)",
        "dbus",
    ]
)

DESCRIPTION = (
    "See how much electricity this computer is using\n"
    " HorsePower reads the energy counters built into Intel and AMD processors\n"
    " and shows the power drawn right now, a breakdown of where it goes, the\n"
    " energy used today and the energy used since installation. A small\n"
    " background service samples the counter every second and keeps a local\n"
    " SQLite log, so totals stay correct while the window is closed. Daily\n"
    " totals are kept forever and minute samples for ninety days. An optional\n"
    " tariff turns energy into cost.\n"
    " .\n"
    " The breakdown separates the CPU package, the graphics card, memory,\n"
    " storage and the rest of the system, omitting any part the machine cannot\n"
    " measure. Readings cover the whole platform where a platform counter is\n"
    " exposed, and the battery on machines without RAPL. Nothing leaves the\n"
    " computer."
)

CHANGELOG = (
    f"{PACKAGE} ({VERSION}) unstable; urgency=medium\n"
    "\n"
    "  * First release: live power with a per-component breakdown, daily and\n"
    "    lifetime totals, one-hour history and cost estimates.\n"
    "\n"
    f" -- {MAINTAINER}  Sun, 01 Nov 2026 12:00:00 +0000\n"
)


def write_gzip(source: Path, target: Path, mtime: int = 0) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    with source.open("rb") as src, gzip.GzipFile(
        filename="", mode="wb", fileobj=target.open("wb"), mtime=mtime
    ) as dst:
        shutil.copyfileobj(src, dst)


def md5sums(stage: Path) -> str:
    lines = []
    for path in sorted(stage.rglob("*")):
        if not path.is_file() or path.is_symlink():
            continue
        digest = hashlib.md5(path.read_bytes()).hexdigest()
        lines.append(f"{digest}  {path.relative_to(stage)}")
    return "\n".join(lines) + "\n"


def installed_size_bytes(stage: Path) -> int:
    return sum(path.stat().st_size for path in stage.rglob("*") if path.is_file())


def build_daemon(target_triple: str | None = None) -> Path:
    """Compile the Rust service in release mode and return the binary path.

    ``cargo`` is located on ``PATH`` or in ``~/.cargo/bin`` so a rustup install
    works without changing the caller's environment.
    """
    cargo = shutil.which("cargo")
    if cargo is None:
        candidate = Path.home() / ".cargo" / "bin" / "cargo"
        if candidate.is_file():
            cargo = str(candidate)
    if cargo is None:
        raise SystemExit(
            "cargo is required to build the daemon. Install Rust with:\n"
            "    curl --proto '=https' --tlsv1.2 https://sh.rustup.rs -sSf | sh -s -- -y\n"
            "or: sudo apt install cargo"
        )

    command = [
        cargo,
        "build",
        "--release",
        "--locked",
        "--manifest-path",
        str(RUST_DIR / "Cargo.toml"),
    ]
    # Use the vendored crates when they are present, so a package build works
    # without network access.
    if (RUST_DIR / "vendor").is_dir():
        command.append("--offline")
    if target_triple:
        command.extend(["--target", target_triple])
    subprocess.run(command, check=True)

    binary = DAEMON_BINARY
    if target_triple:
        binary = RUST_DIR / "target" / target_triple / "release" / "horsepowerd"
    if not binary.is_file():
        raise SystemExit(f"the daemon was not produced at {binary}")
    return binary


def dpkg_architecture() -> str:
    """The Debian architecture this machine builds for."""
    try:
        result = subprocess.run(
            ["dpkg", "--print-architecture"],
            check=True,
            capture_output=True,
            text=True,
        )
        return result.stdout.strip() or "amd64"
    except (OSError, subprocess.CalledProcessError):
        return "amd64"


def stage_tree(
    stage: Path,
    *,
    include_control: bool = True,
    architecture: str | None = None,
) -> None:
    """Install HorsePower's files under *stage* (a future filesystem root)."""
    lib = stage / "usr/lib/horsepower"
    bin_dir = stage / "usr/bin"
    share = stage / "usr/share"

    # Python modules and launchers --------------------------------------
    shutil.copytree(
        ROOT / "src/horsepower",
        lib / "horsepower",
        ignore=shutil.ignore_patterns("__pycache__", "*.pyc"),
    )
    bin_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT / "bin" / "horsepower", bin_dir / "horsepower")
    # The service is a compiled binary, not a script.
    shutil.copy2(build_daemon(), bin_dir / "horsepowerd")

    # Desktop integration -----------------------------------------------
    applications = share / "applications"
    applications.mkdir(parents=True)
    shutil.copy2(
        ROOT / "data/org.horsepower.meter.desktop",
        applications / "org.horsepower.meter.desktop",
    )

    metainfo = share / "metainfo"
    metainfo.mkdir(parents=True)
    shutil.copy2(
        ROOT / "data/org.horsepower.meter.metainfo.xml",
        metainfo / "org.horsepower.meter.metainfo.xml",
    )

    icons = ROOT / "data" / "icons"
    for source in icons.rglob("*"):
        if source.is_file():
            target = share / "icons" / source.relative_to(icons)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)

    # Service, D-Bus policy and activation -------------------------------
    systemd = stage / "usr/lib/systemd/system"
    systemd.mkdir(parents=True)
    shutil.copy2(ROOT / "packaging/horsepower.service", systemd / "horsepower.service")

    dbus_policy = share / "dbus-1/system.d"
    dbus_policy.mkdir(parents=True)
    shutil.copy2(
        ROOT / "packaging/org.horsepower.Daemon.conf",
        dbus_policy / "org.horsepower.Daemon.conf",
    )

    dbus_service = share / "dbus-1/system-services"
    dbus_service.mkdir(parents=True)
    shutil.copy2(
        ROOT / "packaging/org.horsepower.Daemon.service",
        dbus_service / "org.horsepower.Daemon.service",
    )

    # Manual pages -------------------------------------------------------
    write_gzip(ROOT / "docs/horsepower.1", share / "man/man1/horsepower.1.gz")
    write_gzip(ROOT / "docs/horsepowerd.8", share / "man/man8/horsepowerd.8.gz")

    # Machine-readable D-Bus interface, useful for integrators ------------
    interface = share / "horsepower"
    interface.mkdir(parents=True)
    shutil.copy2(
        ROOT / "data/org.horsepower.Daemon.xml",
        interface / "org.horsepower.Daemon.xml",
    )

    # Documentation ------------------------------------------------------
    doc = share / "doc" / PACKAGE
    doc.mkdir(parents=True)
    shutil.copy2(ROOT / "LICENSE", doc / "copyright")
    changelog = doc / "changelog"
    changelog.write_text(CHANGELOG, encoding="utf-8")
    write_gzip(changelog, doc / "changelog.gz")
    changelog.unlink()

    if include_control:
        control_dir = stage / "DEBIAN"
        control_dir.mkdir()
        for script in ("postinst", "prerm", "postrm"):
            shutil.copy2(ROOT / "packaging" / script, control_dir / script)

    # Normalise permissions: never ship group-writable files.
    for path in stage.rglob("*"):
        if path == stage / "DEBIAN":
            continue
        path.chmod(0o755 if path.is_dir() else 0o644)
    for launcher in (bin_dir / "horsepower", bin_dir / "horsepowerd"):
        launcher.chmod(0o755)
    if include_control:
        for script in ("postinst", "prerm", "postrm"):
            (stage / "DEBIAN" / script).chmod(0o755)

    if include_control:
        installed_size = max(1, installed_size_bytes(stage) // 1024)
        control = (
            f"Package: {PACKAGE}\n"
            f"Version: {VERSION}\n"
            f"Architecture: {architecture or dpkg_architecture()}\n"
            f"Maintainer: {MAINTAINER}\n"
            f"Installed-Size: {installed_size}\n"
            f"Depends: {DEPENDS}\n"
            "Section: utils\n"
            "Priority: optional\n"
            f"Homepage: {HOMEPAGE}\n"
            "Description: " + DESCRIPTION + "\n"
        )
        (stage / "DEBIAN" / "control").write_text(control, encoding="utf-8")
        (stage / "DEBIAN" / "md5sums").write_text(md5sums(stage), encoding="utf-8")
