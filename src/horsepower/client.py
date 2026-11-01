"""Asynchronous client for the HorsePower D-Bus service."""

from __future__ import annotations

import logging

import gi

gi.require_version("Gio", "2.0")
gi.require_version("GLib", "2.0")
from gi.repository import Gio, GLib, GObject  # noqa: E402

from .constants import BUS_NAME, INTERFACE, INTROSPECTION_XML, OBJECT_PATH  # noqa: E402

log = logging.getLogger(__name__)


class PowerClient(GObject.Object):
    """Thin, non-blocking proxy around ``org.horsepower.Daemon``.

    Signals:
        status-updated(dict): a fresh status snapshot.
        history-updated(list): minute history points (bucket, watts, wh).
        availability-changed(bool): the service appeared or disappeared.
        history-invalidated(): the log changed; the chart should be reloaded.
    """

    __gsignals__ = {
        "status-updated": (GObject.SignalFlags.RUN_FIRST, None, (object,)),
        "history-updated": (GObject.SignalFlags.RUN_FIRST, None, (object,)),
        "availability-changed": (GObject.SignalFlags.RUN_FIRST, None, (bool,)),
        "history-invalidated": (GObject.SignalFlags.RUN_FIRST, None, ()),
    }

    def __init__(self, bus_type: Gio.BusType = Gio.BusType.SYSTEM):
        super().__init__()
        self._available = False
        self._connection: Gio.DBusConnection | None = None
        self._proxy: Gio.DBusProxy | None = None
        self._history_generation = 0

        try:
            self._connection = Gio.bus_get_sync(bus_type, None)
            node = Gio.DBusNodeInfo.new_for_xml(INTROSPECTION_XML)
            self._proxy = Gio.DBusProxy.new_sync(
                self._connection,
                Gio.DBusProxyFlags.NONE,
                node.interfaces[0],
                BUS_NAME,
                OBJECT_PATH,
                INTERFACE,
                None,
            )
            self._proxy.connect("g-signal", self._on_signal)
            self._proxy.connect("notify::g-name-owner", self._on_name_owner)
        except GLib.Error as exc:
            log.warning("Cannot connect to D-Bus: %s", exc)

        self._available = self._proxy is not None and self._proxy.get_name_owner() is not None

    @property
    def available(self) -> bool:
        return self._available

    def _set_available(self, available: bool) -> None:
        if available != self._available:
            self._available = available
            self.emit("availability-changed", available)

    def _on_name_owner(self, proxy: Gio.DBusProxy, _pspec) -> None:
        available = proxy.get_name_owner() is not None
        self._set_available(available)
        if available:
            self.get_status()
            self.get_history(60)

    def _on_signal(
        self,
        _proxy: Gio.DBusProxy,
        _sender: str,
        signal_name: str,
        parameters: GLib.Variant,
    ) -> None:
        if signal_name == "StatusChanged":
            try:
                (status,) = parameters.unpack()
            except (ValueError, TypeError):
                return
            self._set_available(True)
            self.emit("status-updated", status)
        elif signal_name == "HistoryChanged":
            self.emit("history-invalidated")

    def get_status(self) -> None:
        if self._proxy is None:
            return
        self._proxy.call(
            "GetStatus",
            None,
            Gio.DBusCallFlags.NONE,
            2_000,
            None,
            self._on_status_reply,
        )

    def _on_status_reply(self, proxy: Gio.DBusProxy, result) -> None:
        try:
            variant = proxy.call_finish(result)
            (status,) = variant.unpack()
        except GLib.Error:
            self._set_available(False)
            return
        self._set_available(True)
        self.emit("status-updated", status)

    def get_history(self, minutes: int = 60) -> None:
        if self._proxy is None:
            return
        self._history_generation += 1
        generation = self._history_generation
        self._proxy.call(
            "GetHistory",
            GLib.Variant("(u)", (max(1, int(minutes)),)),
            Gio.DBusCallFlags.NONE,
            2_000,
            None,
            self._on_history_reply,
            generation,
        )

    def _on_history_reply(self, proxy: Gio.DBusProxy, result, generation: int) -> None:
        try:
            variant = proxy.call_finish(result)
            (points,) = variant.unpack()
        except GLib.Error:
            return
        if generation != self._history_generation:
            return
        self.emit("history-updated", list(points))
