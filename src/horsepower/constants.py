"""Shared constants for the service and the graphical client.

The measuring service is written in Rust (see ``rust/horsepowerd``); this
module holds only what the GUI needs to talk to it.
"""

from __future__ import annotations

APP_ID = "org.horsepower.meter"
APP_NAME = "HorsePower"
VERSION = "1.0.0"

#: Bus name owned by the Rust service.
BUS_NAME = "org.horsepower.Daemon"

#: Object path the interface lives on.
OBJECT_PATH = "/org/horsepower/Daemon"

#: Interface name, identical to the bus name.
INTERFACE = BUS_NAME

#: Display order and wording for the per-component breakdown. The service sends
#: a label with every row; this table supplies the sort order the GUI applies.
COMPONENT_LABELS = (
    ("cpu", "CPU"),
    ("gpu", "GPU"),
    ("ram", "Memory"),
    ("storage", "Storage"),
    ("other", "Rest of system"),
    ("system", "Whole system"),
)

#: Minimal introspection XML for the client proxy. Kept in step with
#: ``data/org.horsepower.Daemon.xml`` and the Rust interface.
INTROSPECTION_XML = """
<node>
  <interface name="org.horsepower.Daemon">
    <method name="GetStatus">
      <arg name="status" type="a{sv}" direction="out"/>
    </method>
    <method name="GetHistory">
      <arg name="minutes" type="u" direction="in"/>
      <arg name="points" type="a(xdd)" direction="out"/>
    </method>
    <signal name="StatusChanged">
      <arg name="status" type="a{sv}"/>
    </signal>
    <signal name="HistoryChanged"/>
  </interface>
</node>
"""
