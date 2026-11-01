from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from horsepower.settings import Settings
from horsepower.ui.formatting import (
    format_date,
    format_energy,
    format_money,
    nice_maximum,
)


class SettingsTests(unittest.TestCase):
    def test_defaults(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            settings = Settings.load(Path(tmp) / "missing.json")
            self.assertEqual(settings.currency, "€")
            self.assertGreater(settings.tariff, 0)

    def test_round_trip(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "nested" / "settings.json"
            original = Settings(tariff=0.2745, currency="$")
            original.save(path)
            loaded = Settings.load(path)
            self.assertAlmostEqual(loaded.tariff, 0.2745)
            self.assertEqual(loaded.currency, "$")

    def test_invalid_values_are_clamped(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "settings.json"
            path.write_text(json.dumps({"tariff": 9999, "currency": "   "}), encoding="utf-8")
            settings = Settings.load(path)
            self.assertEqual(settings.tariff, 100.0)
            self.assertEqual(settings.currency, "€")

    def test_broken_file_is_ignored(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "settings.json"
            path.write_text("not json", encoding="utf-8")
            self.assertEqual(Settings.load(path).currency, "€")

    def test_price(self) -> None:
        settings = Settings(tariff=0.30)
        self.assertAlmostEqual(settings.price(1000.0), 0.30)
        self.assertEqual(settings.price(-5.0), 0.0)


class FormattingTests(unittest.TestCase):
    def test_energy_units(self) -> None:
        self.assertEqual(format_energy(412.7), "412.7 Wh")
        self.assertEqual(format_energy(38_914.2), "38.91 kWh")
        self.assertEqual(format_energy(0), "0.0 Wh")

    def test_money(self) -> None:
        self.assertEqual(format_money(0.129, "€"), "€0.13")
        self.assertEqual(format_money(2.5, "USD"), "USD 2.50")

    def test_nice_maximum(self) -> None:
        self.assertEqual(nice_maximum(12), 25.0)
        self.assertEqual(nice_maximum(63), 75.0)
        self.assertEqual(nice_maximum(5000), 5000.0)

    def test_date(self) -> None:
        self.assertEqual(format_date(0), "the first sample")


if __name__ == "__main__":
    unittest.main()
