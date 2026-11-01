"""Colours for the custom-drawn parts of the interface."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class Palette:
    background: str
    surface: str
    ink: str
    muted: str
    faint: str
    accent: str
    accent_text: str
    grid: str

    def rgb(self, value: str) -> tuple[float, float, float]:
        value = value.lstrip("#")
        return tuple(int(value[i : i + 2], 16) / 255.0 for i in (0, 2, 4))  # type: ignore[return-value]

    def rgba(self, value: str, alpha: float) -> tuple[float, float, float, float]:
        r, g, b = self.rgb(value)
        return r, g, b, alpha


DARK = Palette(
    background="#0b0c0b",
    surface="#131511",
    ink="#f0f2e7",
    muted="#858b7d",
    faint="#292d24",
    accent="#c9f75a",
    accent_text="#11140a",
    grid="#20231c",
)

LIGHT = Palette(
    background="#f3f2ec",
    surface="#e9e8df",
    ink="#171910",
    muted="#686d5f",
    faint="#d5d6c9",
    accent="#557600",
    accent_text="#f7f8ef",
    grid="#dcddd0",
)
