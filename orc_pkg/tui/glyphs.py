"""Glyph craft: gradient meters, braille sparklines, compact formatters.

Pure functions — everything here renders from data + theme tokens and is
unit-tested without a running app. This is the TUI's "typography".
"""

from __future__ import annotations

import math

from rich.text import Text

from orc_pkg.tui.theme import Theme, gradient_at

# Fractional fill for the meter's last cell, 1/8 → 7/8.
_PARTIALS = " ▏▎▍▌▋▊▉"
_EMPTY = "╌"
_TICK = "▏"

STATUS_GLYPH = {
    "running": "◉",
    "starting": "◍",
    "done": "●",
    "failed": "✕",
    "killed": "◌",
    "orphaned": "○",
}

BRAIN_GLYPH = {"claude": "🧠", "codex": "🤖", "human": "👤"}


def status_glyph(status: str) -> str:
    return STATUS_GLYPH.get(status, "·")


def meter(pct: float | None, width: int, theme: Theme,
          warn: float = 25, block: float = 10) -> Text:
    """btop-style meter: each filled cell colored by its position along the
    gradient, so a draining bar recedes into the red end. Threshold notches
    at warn/block stay visible over filled and unfilled cells alike."""
    t = Text()
    if pct is None:
        t.append(_EMPTY * (width - 2), style=theme.text_dim)
        t.append(" ?", style=theme.text_dim)
        return t

    pct = max(0.0, min(100.0, float(pct)))
    cells = pct / 100 * width
    filled = int(cells)
    frac = cells - filled
    tick_cells = {int(round(warn / 100 * width)): theme.warn,
                  int(round(block / 100 * width)): theme.err}

    for i in range(width):
        color = gradient_at((i + 0.5) / width, theme.meter_stops)
        if i in tick_cells:
            t.append(_TICK, style=tick_cells[i])
        elif i < filled:
            t.append("█", style=color)
        elif i == filled and frac >= 0.125:
            t.append(_PARTIALS[int(frac * 8)], style=color)
        else:
            t.append(_EMPTY, style=theme.spark_dim)
    return t


# Braille dot bits, bottom row → top row, per column.
_L = (0x40, 0x04, 0x02, 0x01)
_R = (0x80, 0x20, 0x10, 0x08)


def braille_spark(values: list, width: int) -> str:
    """History sparkline: 2 samples per cell × 4 dot rows, right-aligned."""
    n = width * 2
    vals = [max(0.0, float(v)) for v in list(values)[-n:]]
    if not any(vals):
        return "⠀" * width
    vals = [0.0] * (n - len(vals)) + vals
    hi = max(vals) or 1.0
    levels = [0 if v <= 0 else max(1, min(4, math.ceil(v / hi * 4))) for v in vals]
    out = []
    for i in range(0, n, 2):
        code = 0x2800
        for row in range(levels[i]):
            code |= _L[row]
        for row in range(levels[i + 1]):
            code |= _R[row]
        out.append(chr(code))
    return "".join(out)


_BLOCKS = " ▁▂▃▄▅▆▇█"


def block_spark(values: list, width: int) -> str:
    """Coarser block sparkline, one sample per cell, right-aligned."""
    vals = [max(0.0, float(v)) for v in list(values)[-width:]]
    vals = [0.0] * (width - len(vals)) + vals
    hi = max(vals) or 1.0
    return "".join(_BLOCKS[min(8, math.ceil(v / hi * 8))] for v in vals)


def fmt_tokens(n) -> str:
    n = int(n or 0)
    if n >= 1_000_000:
        return f"{n / 1e6:.1f}M"
    if n >= 1_000:
        return f"{n / 1e3:.1f}k"
    return str(n)


def fmt_usd(x) -> str:
    x = float(x or 0)
    if 0 < x < 0.01:
        return f"${x:.4f}"
    return f"${x:.2f}"


def fmt_dur(secs) -> str:
    secs = int(max(0, secs or 0))
    if secs < 60:
        return f"{secs}s"
    if secs < 3600:
        return f"{secs // 60}m{secs % 60:02d}s"
    return f"{secs // 3600}h{(secs % 3600) // 60:02d}m"
