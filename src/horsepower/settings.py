"""User preferences for the GUI (tariff and display currency)."""

from __future__ import annotations

import json
import logging
import os
from dataclasses import asdict, dataclass
from pathlib import Path

log = logging.getLogger(__name__)


def config_path() -> Path:
    base = os.environ.get("XDG_CONFIG_HOME")
    root = Path(base) if base else Path.home() / ".config"
    return root / "horsepower" / "settings.json"


@dataclass
class Settings:
    """Everything the client needs to turn energy into money."""

    tariff: float = 0.30
    currency: str = "€"

    @classmethod
    def load(cls, path: Path | None = None) -> "Settings":
        path = path or config_path()
        try:
            raw = json.loads(path.read_text(encoding="utf-8"))
        except FileNotFoundError:
            return cls()
        except (OSError, ValueError) as exc:
            log.warning("Ignoring unreadable settings at %s: %s", path, exc)
            return cls()

        if not isinstance(raw, dict):
            log.warning("Ignoring invalid settings at %s", path)
            return cls()

        instance = cls()
        try:
            instance.tariff = min(
                max(float(raw.get("tariff", instance.tariff)), 0.0), 100.0
            )
            currency = str(raw.get("currency", instance.currency)).strip()
            instance.currency = currency[:4] or instance.currency
        except (TypeError, ValueError):
            log.warning("Ignoring invalid settings at %s", path)
            return cls()
        return instance

    def save(self, path: Path | None = None) -> None:
        path = path or config_path()
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_suffix(path.suffix + ".tmp")
        payload = json.dumps(asdict(self), indent=2, ensure_ascii=False) + "\n"
        try:
            with temporary.open("w", encoding="utf-8") as handle:
                handle.write(payload)
                handle.flush()
                os.fsync(handle.fileno())
            os.chmod(temporary, 0o600)
            os.replace(temporary, path)
        except OSError as exc:
            log.warning("Could not save settings to %s: %s", path, exc)

    def price(self, wh: float) -> float:
        return max(wh, 0.0) / 1000.0 * max(self.tariff, 0.0)
