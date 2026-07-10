"""Theme tokens for orc top. Every color in the TUI lives here — nowhere else.

Two personalities ship:
  ember    — molten amber on deep charcoal; the project's identity.
  phosphor — CRT monochrome green; committed single-hue terminal romance.

Status colors are never the only encoding: every status renders with a glyph
and the literal word next to it (validated for CVD separation + contrast).
"""

from __future__ import annotations

from dataclasses import dataclass

from orc_pkg import quota

Stops = tuple  # ((frac, "#rrggbb"), ...)


@dataclass(frozen=True)
class Theme:
    name: str
    # canvas
    bg: str
    panel: str
    surface: str
    border: str
    border_focus: str
    # ink
    text: str
    text_dim: str
    label: str
    accent: str
    accent2: str
    # semantics
    ok: str
    warn: str
    err: str
    # run states
    run_running: str
    run_starting: str
    run_done: str
    run_failed: str
    run_killed: str
    run_orphaned: str
    # brains
    brain_claude: str
    brain_codex: str
    brain_human: str
    # instruments
    meter_stops: Stops
    spark: str
    spark_dim: str

    def status_color(self, status: str) -> str:
        return {
            "running": self.run_running,
            "starting": self.run_starting,
            "done": self.run_done,
            "failed": self.run_failed,
            "killed": self.run_killed,
            "orphaned": self.run_orphaned,
        }.get(status, self.text_dim)

    def brain_color(self, brain: str) -> str:
        return {
            "claude": self.brain_claude,
            "codex": self.brain_codex,
            "human": self.brain_human,
        }.get(brain, self.text_dim)


EMBER = Theme(
    name="ember",
    bg="#16120e", panel="#1d1814", surface="#241e18",
    border="#3a2f24", border_focus="#ff9e3d",
    text="#e8ddcf", text_dim="#8a7a66", label="#a08b6f",
    accent="#ff9e3d", accent2="#ffd27a",
    ok="#9ecb5a", warn="#ffb84d", err="#ff5c47",
    run_running="#ffd27a", run_starting="#e8c15a", run_done="#9ecb5a",
    run_failed="#ff5c47", run_killed="#c47ab8", run_orphaned="#7a6a58",
    brain_claude="#e8a1ff", brain_codex="#7ad0ff", brain_human="#c9c9c9",
    meter_stops=((0.0, "#ff3b2f"), (0.10, "#ff5c47"), (0.25, "#ffb84d"),
                 (0.60, "#e8c15a"), (1.0, "#9ecb5a")),
    spark="#ff9e3d", spark_dim="#5c4a36",
)

PHOSPHOR = Theme(
    name="phosphor",
    bg="#050a06", panel="#081108", surface="#0b160c",
    border="#1d3a20", border_focus="#4dff7a",
    text="#b8f0c0", text_dim="#4a7a52", label="#5f9a68",
    accent="#4dff7a", accent2="#c8ffd4",
    ok="#4dff7a", warn="#e8ff5a", err="#ff6b4d",
    run_running="#c8ffd4", run_starting="#8fe8a0", run_done="#4dff7a",
    run_failed="#ff6b4d", run_killed="#9fb8a8", run_orphaned="#4a7a52",
    brain_claude="#9dffb8", brain_codex="#6be8ff", brain_human="#88aa90",
    meter_stops=((0.0, "#ff6b4d"), (0.10, "#ff8f4d"), (0.25, "#e8ff5a"),
                 (1.0, "#4dff7a")),
    spark="#4dff7a", spark_dim="#1d3a20",
)

THEMES = {"ember": EMBER, "phosphor": PHOSPHOR}


def load_theme(cfg: dict | None = None) -> Theme:
    if cfg is None:
        cfg = quota.load_config()
    return THEMES.get(str(cfg.get("theme", "ember")), EMBER)


def _hex_to_rgb(h: str) -> tuple:
    h = h.lstrip("#")
    return int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16)


def lerp_hex(a: str, b: str, t: float) -> str:
    ra, ga, ba = _hex_to_rgb(a)
    rb, gb, bb = _hex_to_rgb(b)
    return "#{:02x}{:02x}{:02x}".format(
        int(ra + (rb - ra) * t), int(ga + (gb - ga) * t), int(ba + (bb - ba) * t))


def gradient_at(frac: float, stops: Stops) -> str:
    """Color at position ``frac`` (0..1) along a multi-stop gradient."""
    if frac <= stops[0][0]:
        return stops[0][1]
    for (p0, c0), (p1, c1) in zip(stops, stops[1:]):
        if frac <= p1:
            span = (p1 - p0) or 1.0
            return lerp_hex(c0, c1, (frac - p0) / span)
    return stops[-1][1]
