"""Number and date formatting shared by the widgets."""

from __future__ import annotations

from datetime import datetime

_NICE_STEPS = (
    25,
    50,
    75,
    100,
    125,
    150,
    200,
    250,
    300,
    400,
    500,
    750,
    1000,
    1500,
    2000,
    3000,
    5000,
)


def format_energy(wh: float) -> str:
    wh = max(0.0, float(wh))
    if wh >= 1000.0:
        return f"{wh / 1000.0:,.2f} kWh"
    return f"{wh:,.1f} Wh"


def format_money(value: float, currency: str) -> str:
    value = max(0.0, float(value))
    symbol = (currency or "€").strip()
    separator = " " if len(symbol) > 1 else ""
    return f"{symbol}{separator}{value:,.2f}"


def format_power(watts: float) -> str:
    return f"{max(0.0, watts):,.0f} W"


def format_date(timestamp: float | int) -> str:
    if not timestamp:
        return "the first sample"
    return datetime.fromtimestamp(float(timestamp)).strftime("%d %b %Y").lstrip("0")


def nice_maximum(value: float) -> float:
    value = max(float(value), 1.0)
    for step in _NICE_STEPS:
        if step >= value:
            return float(step)
    return float(_NICE_STEPS[-1])
