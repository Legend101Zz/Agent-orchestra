"""Hand-rolled box-drawing DAG: brains on the left rail, worker nodes fanning
right, trunk + elbow connectors. Renders whatever brain topology the metas
carry — N brains become N rails, nothing is hardcoded to two levels."""

from __future__ import annotations

from datetime import datetime, timezone

from rich.cells import cell_len
from rich.text import Text

from orc_pkg.tui.glyphs import BRAIN_GLYPH, fmt_dur, fmt_tokens, fmt_usd, status_glyph
from orc_pkg.tui.theme import Theme


def _run_duration(m: dict) -> float | None:
    try:
        start = datetime.fromisoformat(m["started_at"])
    except (KeyError, TypeError, ValueError):
        return None
    end = None
    if m.get("ended_at"):
        try:
            end = datetime.fromisoformat(m["ended_at"])
        except (TypeError, ValueError):
            end = None
    if end is None:
        end = datetime.now(timezone.utc)
    return max(0.0, (end - start).total_seconds())


def _node_label(m: dict) -> str:
    rid = str(m.get("id", "?"))
    short = rid if len(rid) <= 26 else rid[:25] + "…"
    tokens = m.get("tokens") or {}
    total = tokens.get("total") or tokens.get("estimated_total")
    bits = [short]
    stats = []
    if total:
        prefix = "" if tokens.get("total") else "~"
        stats.append(prefix + fmt_tokens(total))
    if tokens.get("cost_usd"):
        stats.append(fmt_usd(tokens["cost_usd"]))
    dur = _run_duration(m)
    if dur is not None:
        stats.append(fmt_dur(dur))
    if stats:
        bits.append(" · ".join(stats))
    return "  ".join(bits)


def render_flow(runs: list, theme: Theme, width: int = 80,
                selected: str | None = None) -> Text:
    """``selected`` is a run id whose node gets the focus border color."""
    if not runs:
        return Text("∅  no runs in this session yet", style=theme.text_dim)

    groups: dict = {}
    for m in runs:
        if isinstance(m, dict):
            groups.setdefault(m.get("brain", "human"), []).append(m)

    out = Text()
    first_group = True
    for brain, members in groups.items():
        if not first_group:
            out.append("\n")
        first_group = False
        out.append_text(_render_group(brain, members, theme, selected))
    return out


def _render_group(brain: str, members: list, theme: Theme,
                  selected: str | None) -> Text:
    icon = BRAIN_GLYPH.get(brain, "◇")
    brain_label = f"{icon} {brain}"
    brain_w = cell_len(brain_label) + 2

    n = len(members)
    mids = [3 * i + 1 for i in range(n)]
    first_mid, last_mid = mids[0], mids[-1]
    brain_line = (first_mid + last_mid) // 2

    bcolor = theme.brain_color(brain)
    out = Text()
    for i, m in enumerate(members):
        status = str(m.get("status", "?"))
        scolor = theme.border_focus if (selected and m.get("id") == selected) \
            else theme.status_color(status)
        stats = _node_label(m)
        inner_w = cell_len(f"{status_glyph(status)} {status}  {stats}") + 2

        for j in range(3):                       # box line 0/1/2
            line_no = 3 * i + j
            is_mid = line_no in mids
            is_brain = line_no == brain_line
            on_trunk = n > 1 and first_mid <= line_no <= last_mid

            # left rail: brain label on its line, else padding
            if is_brain:
                out.append(" " + brain_label + " ", style=f"bold {bcolor}")
            else:
                out.append(" " * brain_w)

            # connector zone, 3 cells: lead · trunk · dash
            lead = "─" if is_brain else " "
            if n == 1:
                trunk = "─" if is_mid else " "
            elif is_mid and line_no == first_mid:
                trunk = "┼" if is_brain else "╭"
            elif is_mid and line_no == last_mid:
                trunk = "┼" if is_brain else "╰"
            elif is_mid:
                trunk = "┼" if is_brain else "├"
            elif on_trunk:
                trunk = "┤" if is_brain else "│"
            else:
                trunk = " "
            dash = "─" if is_mid else " "
            out.append(lead + trunk + dash, style=theme.border)

            # the node box
            if j == 0:
                out.append("╭" + "─" * inner_w + "╮", style=scolor)
            elif j == 2:
                out.append("╰" + "─" * inner_w + "╯", style=scolor)
            else:
                out.append("┤" if is_mid else "│", style=scolor)
                out.append(" ")
                out.append(status_glyph(status) + " " + status,
                           style=f"bold {scolor}")
                out.append("  " + stats + " ", style=theme.text)
                out.append("│", style=scolor)
            out.append("\n")
    return out
