"""A very small preferences window."""

from __future__ import annotations

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, GLib, Gtk  # noqa: E402

from ..settings import Settings  # noqa: E402


class PreferencesDialog(Adw.PreferencesDialog):
    def __init__(
        self,
        settings: Settings,
        retention_days: int,
        on_changed,
    ):
        super().__init__()
        self._settings = settings
        self._on_changed = on_changed
        self._save_source = 0

        self.set_title("Preferences")
        self.set_search_enabled(False)

        page = Adw.PreferencesPage()
        page.set_title("Meter")
        page.set_icon_name("preferences-system-symbolic")

        billing = Adw.PreferencesGroup()
        billing.set_title("Electricity tariff")
        billing.set_description(
            "Used to estimate the cost of the energy logged on this computer. "
            "Nothing is uploaded."
        )

        price_row = Adw.ActionRow()
        price_row.set_title("Price per kWh")
        price_row.set_subtitle("Check your latest electricity bill")
        self._price = Gtk.SpinButton.new_with_range(0.0, 10.0, 0.01)
        self._price.set_digits(3)
        self._price.set_numeric(True)
        self._price.set_value(settings.tariff)
        self._price.set_valign(Gtk.Align.CENTER)
        self._price.set_width_chars(6)
        self._price.connect("value-changed", self._changed)
        price_row.add_suffix(self._price)
        billing.add(price_row)

        currency_row = Adw.ActionRow()
        currency_row.set_title("Currency")
        currency_row.set_subtitle("A symbol or a short code")
        self._currency = Gtk.Entry()
        self._currency.set_text(settings.currency)
        self._currency.set_max_length(4)
        self._currency.set_width_chars(4)
        self._currency.set_alignment(0.5)
        self._currency.set_valign(Gtk.Align.CENTER)
        self._currency.connect("changed", self._changed)
        currency_row.add_suffix(self._currency)
        billing.add(currency_row)

        page.add(billing)

        data = Adw.PreferencesGroup()
        data.set_title("Data")
        data.set_description(
            "The background service writes to /var/lib/horsepower/power.db. "
            "It starts automatically at boot."
        )
        retention = Adw.ActionRow()
        retention.set_title("Minute samples")
        retention.set_subtitle(f"Kept for {retention_days} days")
        data.add(retention)
        totals = Adw.ActionRow()
        totals.set_title("Daily totals")
        totals.set_subtitle("Kept for the lifetime of the installation")
        data.add(totals)
        page.add(data)

        self.add(page)
        self.connect("closed", self._on_closed)

    def _changed(self, *_args) -> None:
        self._settings.tariff = round(self._price.get_value(), 4)
        currency = self._currency.get_text().strip()[:4]
        if currency:
            self._settings.currency = currency
        self._on_changed(self._settings)
        if self._save_source:
            GLib.source_remove(self._save_source)
        self._save_source = GLib.timeout_add(450, self._save)

    def _save(self) -> bool:
        self._save_source = 0
        self._settings.save()
        return GLib.SOURCE_REMOVE

    def _on_closed(self, *_args) -> None:
        if self._save_source:
            GLib.source_remove(self._save_source)
            self._save_source = 0
        self._settings.save()
