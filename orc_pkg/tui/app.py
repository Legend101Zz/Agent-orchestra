"""orc top — the pi-orchestra control plane.

btop-energy instrument cluster: gradient quota meters with history, metric
tiles (delegated-value hero), a 24h activity strip, a session tree-table and
a live log tail. Read-only against the registry; kills and new tasks shell
out to `python -m orc_pkg`.
"""

from __future__ import annotations

import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

from rich.text import Text
from textual import work
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal
from textual.theme import Theme as TextualTheme
from textual.widgets import DataTable, Input, RichLog, Static

from orc_pkg import metrics, quota, registry
from orc_pkg.tui import glyphs as G
from orc_pkg.tui.grouping import flatten, group_sessions
from orc_pkg.tui.theme import THEMES, Theme, load_theme

SORT_MODES = ("recent", "tokens", "cost", "status")
_STATUS_ORDER = {"running": 0, "starting": 1, "failed": 2, "killed": 3,
                 "orphaned": 4, "done": 5}


def spaced(s: str) -> str:
    """Letter-spaced caps — the label voice of the whole UI."""
    return " ".join(s.upper())


def _age(ts: float | None) -> str:
    if not ts:
        return "—"
    return G.fmt_dur(time.time() - ts)


def _run_elapsed(m: dict) -> str:
    try:
        start = datetime.fromisoformat(m["started_at"])
    except (KeyError, TypeError, ValueError):
        return "—"
    if m.get("ended_at"):
        try:
            end = datetime.fromisoformat(m["ended_at"])
        except (TypeError, ValueError):
            return "—"
    elif m.get("status") in ("running", "starting"):
        end = datetime.now(timezone.utc)
    else:
        return "—"          # orphaned with no recorded end: duration unknown
    return G.fmt_dur((end - start).total_seconds())


class HeaderBar(Static):
    def render(self) -> Text:
        app = self.app
        th: Theme = app.tokens
        t = Text()
        t.append(" ▞▚ ", style=f"bold {th.accent}")
        t.append("O R C", style=f"bold {th.text}")
        t.append("  pi-orchestra", style=th.text_dim)
        t.append("   ")
        for status in ("running", "done", "failed", "killed"):
            n = app.status_counts.get(status, 0)
            if status == "running" or n:
                t.append(f" {G.status_glyph(status)}{n}",
                         style=th.status_color(status))
        pad = max(1, self.size.width - t.cell_len - 22)
        t.append(" " * pad)
        t.append(datetime.now().strftime("%H:%M:%S "), style=th.text_dim)
        t.append(f"◈ {th.name}", style=th.accent)
        return t


class QuotaPanel(Static):
    def render(self) -> Text:
        app = self.app
        th: Theme = app.tokens
        q = app.quota_state
        hist = app.quota_history
        cfg = app.cfg
        warn, block = cfg.get("warn_pct", 25), cfg.get("block_pct", 10)
        width = max(12, self.size.width - 16)
        t = Text()
        if q is None:
            t.append("tuning…", style=th.text_dim)
            return t
        if q.get("level") == "unknown":
            t.append("● UNKNOWN", style=f"bold {th.warn}")
            t.append(f"  {q.get('reason', '')[:60]}\n", style=th.text_dim)
            t.append("delegation not gated — proceed with judgement",
                     style=th.text_dim)
            return t
        five, weekly = q.get("five_hour_pct"), q.get("weekly_pct")
        t.append(" 5H ", style=th.label)
        t.append_text(G.meter(five, width, th, warn, block))
        t.append(f" {five:>3.0f}%", style=th.text)
        t.append(f" ↻{q.get('window_resets_in_min', '?')}m\n", style=th.text_dim)
        t.append(" WK ", style=th.label)
        t.append_text(G.meter(weekly, width, th, warn, block))
        t.append(f" {weekly:>3.0f}%\n", style=th.text)
        vals = [h.get("five_hour_pct") or 0 for h in hist]
        t.append(" ")
        t.append(G.braille_spark(vals, max(8, width // 2)), style=th.spark)
        t.append(f"  5h history · {len(vals)} samples\n", style=th.text_dim)
        level = q.get("level", "?")
        lcolor = {"ok": th.ok, "warn": th.warn, "block": th.err}.get(level, th.text_dim)
        t.append(" ● ", style=lcolor)
        t.append(level.upper(), style=f"bold {lcolor}")
        t.append(f"   warn ≤{warn}% · block ≤{block}%", style=th.text_dim)
        return t


class Tile(Static):
    def __init__(self, tile_id: str, title: str):
        super().__init__(id=tile_id, classes="tile")
        self.border_title = spaced(title)

    def render(self) -> Text:
        return self.app.tile_text(self.id)


class ActivityStrip(Static):
    def render(self) -> Text:
        app = self.app
        th: Theme = app.tokens
        buckets = app.activity_buckets
        t = Text()
        t.append(" " + spaced("activity") + " ", style=th.label)
        w = max(12, self.size.width - 36)
        t.append(G.braille_spark(buckets, w), style=th.spark)
        total = int(sum(buckets))
        t.append(f"  {total} runs · 24h", style=th.text_dim)
        return t


class FooterBar(Static):
    KEYS = (("j/k", "nav"), ("enter", "open"), ("x", "kill"), ("n", "new"),
            ("s", "sort"), ("/", "filter"), ("t", "theme"), ("?", "help"),
            ("q", "quit"))

    def render(self) -> Text:
        th: Theme = self.app.tokens
        t = Text(" ")
        for i, (key, label) in enumerate(self.KEYS):
            if i:
                t.append(" · ", style=th.text_dim)
            t.append(key, style=f"bold {th.accent}")
            t.append(f" {label}", style=th.text_dim)
        f = self.app.filter_text
        if f:
            t.append(f"   ⌕ {f}", style=f"bold {th.warn}")
        t.append(f"   ⇅ {self.app.sort_mode}", style=th.text_dim)
        return t


class HelpOverlay(Static):
    TEXT = """\
 j / k / ↑ / ↓      move through sessions and runs
 enter / click      expand a session · open a run
 x                  kill selected (press twice to confirm)
 n                  launch a new MiniMax worker
 s                  cycle sort: recent → tokens → cost → status
 /                  filter by id, task or session
 t                  switch theme (saved to config)
 r                  refresh now
 esc                back / close
 q                  quit

 Exact tokens come from pi's agent_end usage; ~ marks
 chars/4 estimates. Delegated value = worker tokens
 priced at brain API rates minus what MiniMax charged."""

    def render(self) -> Text:
        th: Theme = self.app.tokens
        t = Text(spaced("keys & honesty") + "\n\n", style=f"bold {th.accent}")
        t.append(self.TEXT, style=th.text)
        return t


class OrcTop(App):
    TITLE = "orc top — pi-orchestra control plane"

    CSS = """
    Screen { background: $orc-bg; color: $orc-text; layers: base help; }
    * {
        scrollbar-background: $orc-panel; scrollbar-color: $orc-border;
        scrollbar-color-hover: $orc-accent; scrollbar-size-vertical: 1;
    }
    #hdr { height: 1; padding: 0; }
    #top { height: 7; }
    #quota-panel {
        width: 46%; max-width: 74; border: round $orc-border;
        border-title-color: $orc-accent; background: $orc-panel; padding: 0 1;
    }
    .tile {
        border: round $orc-border; border-title-color: $orc-label;
        background: $orc-panel; width: 1fr; margin-left: 1; padding: 0 1;
    }
    #tile-value { border: round $orc-accent 60%; }
    #activity { height: 1; padding: 0 1; }
    #runs-table {
        border: round $orc-border; border-title-color: $orc-accent;
        background: $orc-bg; height: 1fr;
    }
    #runs-table:focus { border: round $orc-border-focus; }
    #log-tail {
        height: 9; border: round $orc-border; border-title-color: $orc-label;
        background: $orc-panel; padding: 0 1;
    }
    #ftr { height: 1; }
    #new-task, #filter-box { dock: bottom; display: none; }
    #new-task.visible, #filter-box.visible { display: block; }
    #help-wrap {
        layer: help; width: 100%; height: 100%;
        align: center middle; display: none;
    }
    #help-wrap.visible { display: block; }
    #help {
        width: 62; padding: 1 2; border: round $orc-accent;
        background: $orc-panel;
    }
    """

    BINDINGS = [
        Binding("q", "quit", "quit"),
        Binding("j", "nav(1)", "down", show=False),
        Binding("k", "nav(-1)", "up", show=False),
        Binding("x", "kill_selected", "kill"),
        Binding("n", "new_task", "new"),
        Binding("s", "cycle_sort", "sort"),
        Binding("slash", "filter", "filter", show=False),
        Binding("t", "cycle_theme", "theme"),
        Binding("question_mark", "help", "help", show=False),
        Binding("r", "refresh_now", "refresh"),
    ]

    def __init__(self, theme_name: str | None = None):
        self.cfg = quota.load_config()
        if theme_name:
            self.cfg["theme"] = theme_name
        self.tokens: Theme = load_theme(self.cfg)
        super().__init__()
        self.quota_state: dict | None = None
        self.quota_history: list = []
        self.brain_state: dict = {"claude": None, "codex": None}
        self.status_counts: dict = {}
        self.activity_buckets: list = [0.0] * 48
        self.sort_mode = "recent"
        self.filter_text = ""
        self._runs: list = []
        self._visible: list = []
        self._expanded: set = set()
        self._confirm_kill: str | None = None
        self._tail_key = None

    # ---- theme plumbing -------------------------------------------------
    def get_css_variables(self) -> dict:
        th: Theme = getattr(self, "tokens", THEMES["ember"])
        v = super().get_css_variables()
        v.update({
            "orc-bg": th.bg, "orc-panel": th.panel, "orc-surface": th.surface,
            "orc-border": th.border, "orc-border-focus": th.border_focus,
            "orc-text": th.text, "orc-label": th.label, "orc-accent": th.accent,
        })
        return v

    def _register_themes(self) -> None:
        for th in THEMES.values():
            self.register_theme(TextualTheme(
                name=th.name, primary=th.accent, secondary=th.accent2,
                warning=th.warn, error=th.err, success=th.ok, accent=th.accent,
                foreground=th.text, background=th.bg, surface=th.surface,
                panel=th.panel, dark=True,
            ))

    # ---- layout ----------------------------------------------------------
    def compose(self) -> ComposeResult:
        yield HeaderBar(id="hdr")
        with Horizontal(id="top"):
            yield QuotaPanel(id="quota-panel")
            yield Tile("tile-value", "saved")
            yield Tile("tile-tokens", "tokens")
            yield Tile("tile-cost", "cost")
            yield Tile("tile-active", "active")
        yield ActivityStrip(id="activity")
        yield DataTable(id="runs-table", zebra_stripes=True)
        yield RichLog(id="log-tail", wrap=True, highlight=False, markup=False)
        yield FooterBar(id="ftr")
        yield Input(placeholder="task for a new MiniMax worker — enter launches, esc cancels",
                    id="new-task")
        yield Input(placeholder="filter runs — esc clears", id="filter-box")
        with Static(id="help-wrap"):
            yield HelpOverlay(id="help")

    def on_mount(self) -> None:
        self._register_themes()
        self.theme = self.tokens.name
        panel = self.query_one("#quota-panel", QuotaPanel)
        panel.border_title = spaced("quota · minimax")
        table = self.query_one("#runs-table", DataTable)
        table.cursor_type = "row"
        table.add_columns("", "SESSION / RUN", "STATUS", "BRAIN",
                          "ELAPSED", "TOK", "COST", "TASK")
        table.focus()
        self.query_one("#log-tail", RichLog).border_title = spaced("log")
        self.refresh_data()
        self._fetch_quota()
        self._fetch_brains()
        self.set_interval(2.0, self.refresh_data)
        self.set_interval(1.0, lambda: self.query_one("#hdr", HeaderBar).refresh())
        self.set_interval(60.0, self._fetch_quota)
        self.set_interval(60.0, self._fetch_brains)

    # ---- background fetchers ---------------------------------------------
    @work(thread=True, exclusive=True, group="quota")
    def _fetch_quota(self) -> None:
        q = quota.get_quota()
        hist = quota.read_history(limit=96)
        self.call_from_thread(self._apply_quota, q, hist)

    def _apply_quota(self, q: dict, hist: list) -> None:
        self.quota_state = q
        self.quota_history = hist
        self.query_one("#quota-panel", QuotaPanel).refresh()

    @work(thread=True, exclusive=True, group="brains")
    def _fetch_brains(self) -> None:
        b = metrics.brain_usage()
        self.call_from_thread(self._apply_brains, b)

    def _apply_brains(self, b: dict) -> None:
        self.brain_state = b
        self.query_one("#tile-tokens", Tile).refresh()

    # ---- data refresh ------------------------------------------------------
    def refresh_data(self) -> None:
        self._runs = registry.list_runs()
        self.status_counts = {}
        for m in self._runs:
            s = m.get("status", "?")
            self.status_counts[s] = self.status_counts.get(s, 0) + 1
        self._recompute_activity()
        self._rebuild_table()
        for wid in ("#hdr", "#activity", "#tile-value", "#tile-cost",
                    "#tile-active", "#ftr"):
            self.query_one(wid).refresh()
        self._refresh_tail()

    def _recompute_activity(self) -> None:
        now = time.time()
        buckets = [0.0] * 48
        for m in self._runs:
            ts = m.get("created_ts")
            if not ts:
                continue
            age = now - float(ts)
            if 0 <= age < 24 * 3600:
                buckets[47 - int(age // 1800)] += 1
        self.activity_buckets = buckets

    def _match(self, m: dict) -> bool:
        if not self.filter_text:
            return True
        f = self.filter_text.lower()
        return (f in str(m.get("id", "")).lower()
                or f in str(m.get("task", "")).lower()
                or f in str(m.get("session", "")).lower())

    def _sorted_groups(self, groups: list) -> list:
        mode = self.sort_mode
        if mode == "recent":
            return groups

        def keyfor(row):
            meta = row.get("meta", {})
            if mode == "tokens":
                if row["kind"] == "session":
                    return -row["tokens"]
                tk = meta.get("tokens") or {}
                return -(tk.get("total") or tk.get("estimated_total") or 0)
            if mode == "cost":
                if row["kind"] == "session":
                    return -row["cost_usd"]
                return -((meta.get("tokens") or {}).get("cost_usd") or 0)
            status = row["status"] if row["kind"] == "session" else meta.get("status", "?")
            return _STATUS_ORDER.get(status, 9)

        return sorted(groups, key=keyfor)

    def _rebuild_table(self) -> None:
        th = self.tokens
        table = self.query_one("#runs-table", DataTable)
        table.border_title = spaced("sessions · runs")
        prev = table.cursor_row
        table.clear()

        runs = [m for m in self._runs if self._match(m)]
        groups = self._sorted_groups(group_sessions(runs))
        self._visible = flatten(groups, self._expanded)

        for row in self._visible:
            if row["kind"] == "session":
                key = row["key"]
                caret = "▾" if key in self._expanded else "▸"
                scolor = th.status_color(row["status"])
                brains = Text()
                for b in sorted(row["brains"]):
                    brains.append(G.BRAIN_SIGIL.get(b, "◇") + " ",
                                  style=th.brain_color(b))
                table.add_row(
                    Text(caret, style=f"bold {th.accent}"),
                    Text(f"⧉ {key}", style=f"bold {th.accent2}"),
                    Text(f"{G.status_glyph(row['status'])} {row['status']}",
                         style=f"bold {scolor}"),
                    brains,
                    Text(_age(row["started_ts"]), style=th.text_dim,
                         justify="right"),
                    Text(G.fmt_tokens(row["tokens"]), justify="right",
                         style=th.text),
                    Text(G.fmt_usd(row["cost_usd"]), justify="right",
                         style=th.text),
                    Text(f"{row['n']} runs", style=th.text_dim),
                    key=f"s::{key}",
                )
            else:
                m = row["meta"]
                member = row["kind"] == "member"
                rail = ("╰─" if row.get("last") else "├─") if member else ""
                status = m.get("status", "?")
                scolor = th.status_color(status)
                tk = m.get("tokens") or {}
                total = tk.get("total") or tk.get("estimated_total") or 0
                approx = "" if tk.get("total") else "~"
                rid = str(m.get("id", "?"))
                table.add_row(
                    Text(rail, style=th.border),
                    Text(rid[:36], style=th.text if not member else th.text_dim),
                    Text(f"{G.status_glyph(status)} {status}",
                         style=f"bold {scolor}"),
                    Text(f"{G.BRAIN_SIGIL.get(m.get('brain'), '◇')} {m.get('brain', '?')}",
                         style=th.brain_color(m.get("brain", ""))),
                    Text(_run_elapsed(m), justify="right",
                         style=th.accent2 if status == "running" else th.text_dim),
                    Text(approx + G.fmt_tokens(total), justify="right", style=th.text),
                    Text(G.fmt_usd(tk.get("cost_usd")) if tk.get("cost_usd") else "—",
                         justify="right", style=th.text),
                    Text(str(m.get("task", ""))[:56], style=th.text_dim),
                    key=f"r::{rid}",
                )
        if self._visible and prev is not None:
            table.move_cursor(row=min(prev, len(self._visible) - 1))

    # ---- tiles -------------------------------------------------------------
    def tile_text(self, tile_id: str) -> Text:
        th = self.tokens
        t = Text()
        if tile_id == "tile-value":
            dv = metrics.delegated_value(self._runs)
            t.append(f"{G.fmt_usd(dv['saved_usd'])}\n",
                     style=f"bold {th.accent2}")
            mult = dv["multiple"]
            t.append(f"{mult}× vs brain rates\n" if mult else "—\n",
                     style=th.accent)
            t.append(f"{dv['exact_share'] * 100:.0f}% exact basis",
                     style=th.text_dim)
        elif tile_id == "tile-tokens":
            today = datetime.now(timezone.utc).strftime("%Y-%m-%d")
            ws = metrics.worker_stats(self._runs)
            wtok = ws["by_day"].get(today, {}).get("total", 0)
            t.append(f"{G.fmt_tokens(wtok)}\n", style=f"bold {th.text}")
            for name, icon in (("claude", "🧠"), ("codex", "🤖")):
                u = self.brain_state.get(name)
                if u:
                    d = u["today"]
                    val = G.fmt_tokens(d["input"] + d["output"])
                else:
                    val = "n/a"
                t.append(f"{icon} {val}  ", style=th.text_dim)
            t.append("\nworkers · brains", style=th.text_dim)
        elif tile_id == "tile-cost":
            today = datetime.now(timezone.utc).strftime("%Y-%m-%d")
            todays = [m for m in self._runs
                      if str(m.get("started_at", ""))[:10] == today]
            dv = metrics.delegated_value(todays)
            ws = metrics.worker_stats(todays)
            t.append(f"{G.fmt_usd(dv['worker_cost_usd'])}\n",
                     style=f"bold {th.text}")
            t.append(f"MiniMax · {ws['runs']} runs\n", style=th.text_dim)
            est = ws["estimated"]["runs"]
            t.append(f"{est} est." if est else "all exact", style=th.text_dim)
        elif tile_id == "tile-active":
            n = self.status_counts.get("running", 0) + \
                self.status_counts.get("starting", 0)
            color = th.run_running if n else th.text_dim
            t.append(f"◉ {n}\n", style=f"bold {color}")
            t.append(f"of {self.cfg.get('max_parallel_workers', 3)} max\n",
                     style=th.text_dim)
            t.append(G.block_spark(self.activity_buckets[-12:], 12),
                     style=th.spark_dim)
        return t

    # ---- log tail -----------------------------------------------------------
    def _selected(self) -> dict | None:
        table = self.query_one("#runs-table", DataTable)
        if not self._visible or table.cursor_row is None:
            return None
        if 0 <= table.cursor_row < len(self._visible):
            return self._visible[table.cursor_row]
        return None

    def _selected_run_meta(self) -> dict | None:
        row = self._selected()
        if not row:
            return None
        if row["kind"] == "session":
            running = [m for m in row["runs"] if m.get("status") == "running"]
            return (running or row["runs"])[0] if row["runs"] else None
        return row["meta"]

    def _refresh_tail(self) -> None:
        m = self._selected_run_meta()
        log = self.query_one("#log-tail", RichLog)
        th = self.tokens
        if not m:
            if self._tail_key is not None:
                log.clear()
                self._tail_key = None
            log.border_title = spaced("log")
            return
        path = Path(m.get("_dir", "")) / "output.log"
        try:
            stat = path.stat()
            key = (m["id"], stat.st_mtime, stat.st_size)
        except OSError:
            key = (m["id"], None, None)
        if key == self._tail_key:
            return
        self._tail_key = key
        log.clear()
        log.border_title = spaced("log") + f" ╱ {m['id'][:40]}"
        from orc_pkg.tui.convo import parse_log
        parsed = parse_log(path)
        body = parsed["plain"] if parsed["plain"] is not None else parsed["reply"]
        if not body:
            log.write(Text("(no output yet)", style=th.text_dim))
            return
        for line in body.splitlines()[-60:]:
            log.write(line)

    # ---- actions ----------------------------------------------------------
    def action_nav(self, delta: int) -> None:
        table = self.query_one("#runs-table", DataTable)
        if table.row_count:
            cur = table.cursor_row or 0
            table.move_cursor(row=max(0, min(table.row_count - 1, cur + delta)))

    def action_refresh_now(self) -> None:
        self.refresh_data()
        self._fetch_quota()
        self._fetch_brains()

    def action_cycle_sort(self) -> None:
        i = SORT_MODES.index(self.sort_mode)
        self.sort_mode = SORT_MODES[(i + 1) % len(SORT_MODES)]
        self.refresh_data()

    def action_cycle_theme(self) -> None:
        names = list(THEMES)
        i = names.index(self.tokens.name)
        self.tokens = THEMES[names[(i + 1) % len(names)]]
        self.theme = self.tokens.name
        try:
            cfg_path = registry.home() / "config.json"
            cfg = quota.load_config()
            cfg["theme"] = self.tokens.name
            registry.atomic_write_json(cfg_path, cfg)
        except OSError:
            pass
        self.refresh_data()
        self.query_one("#quota-panel", QuotaPanel).refresh()

    def action_help(self) -> None:
        self.query_one("#help-wrap").toggle_class("visible")

    def action_filter(self) -> None:
        box = self.query_one("#filter-box", Input)
        box.add_class("visible")
        box.focus()

    def action_new_task(self) -> None:
        box = self.query_one("#new-task", Input)
        box.add_class("visible")
        box.focus()

    def action_kill_selected(self) -> None:
        row = self._selected()
        if not row:
            return
        if row["kind"] == "session":
            targets = [m["id"] for m in row["runs"]
                       if m.get("status") in ("running", "starting")]
            label = f"{len(targets)} running in {row['key'][:24]}"
        else:
            targets = [row["meta"]["id"]]
            label = row["meta"]["id"][:32]
        if not targets:
            self.notify("nothing running to kill", severity="warning")
            return
        confirm_key = "|".join(targets)
        if self._confirm_kill != confirm_key:
            self._confirm_kill = confirm_key
            self.notify(f"press x again to kill {label}", timeout=3)
            return
        self._confirm_kill = None
        for rid in targets:
            subprocess.run([sys.executable, "-m", "orc_pkg", "kill", rid],
                           capture_output=True)
        self.notify(f"kill sent · {label}")
        self.refresh_data()

    def action_open(self) -> None:
        row = self._selected()
        if not row:
            return
        if row["kind"] == "session":
            key = row["key"]
            self._expanded.symmetric_difference_update({key})
            self._rebuild_table()
            return
        self._open_run(row["meta"])

    def _open_run(self, m: dict) -> None:
        from orc_pkg.tui.session_screen import SessionScreen
        sess = m.get("session")
        if sess:
            members = [r for r in self._runs if r.get("session") == sess]
            title = sess
        else:
            members, title = [m], m.get("id", "run")
        self.push_screen(SessionScreen(members, title, focus_id=m.get("id")))

    # ---- events -----------------------------------------------------------
    def on_data_table_row_selected(self, event) -> None:
        self.action_open()

    def on_data_table_row_highlighted(self, event) -> None:
        self._refresh_tail()

    def on_input_submitted(self, event: Input.Submitted) -> None:
        box = event.input
        value = box.value.strip()
        if box.id == "new-task":
            box.value = ""
            box.remove_class("visible")
            self.query_one("#runs-table", DataTable).focus()
            if value:
                subprocess.run([sys.executable, "-m", "orc_pkg", "run", value,
                                "--bg", "--brain", "human"], capture_output=True)
                self.notify("worker launched")
                self.refresh_data()
        elif box.id == "filter-box":
            self.filter_text = value
            box.remove_class("visible")
            self.query_one("#runs-table", DataTable).focus()
            self.refresh_data()

    def on_key(self, event) -> None:
        help_wrap = self.query_one("#help-wrap")
        if help_wrap.has_class("visible") and event.key != "question_mark":
            help_wrap.remove_class("visible")
            event.stop()
            return
        if event.key == "escape":
            for wid in ("#new-task", "#filter-box"):
                box = self.query_one(wid, Input)
                if box.has_class("visible"):
                    if wid == "#filter-box":
                        box.value = ""
                        self.filter_text = ""
                        self.refresh_data()
                    box.remove_class("visible")
                    self.query_one("#runs-table", DataTable).focus()
                    event.stop()
                    return


def run_tui() -> None:
    OrcTop().run()
