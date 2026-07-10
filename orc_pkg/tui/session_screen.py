"""Session drill-in screen: Flow · Conversation · Log · Meta tabs.

Posting-energy: app-like tabbed detail with a breadcrumb header, `esc` back,
`[`/`]` to walk the session's runs, live log tail with search, markdown
conversation with collapsed thinking.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

from rich.text import Text
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import VerticalScroll
from textual.screen import Screen
from textual.widgets import Input, Markdown, RichLog, Static, TabbedContent, TabPane

from orc_pkg.tui import glyphs as G
from orc_pkg.tui.convo import parse_log
from orc_pkg.tui.flow import render_flow


def _spaced(s: str) -> str:
    return " ".join(s.upper())


class Crumb(Static):
    def render(self) -> Text:
        th = self.app.tokens
        scr = self.screen
        t = Text()
        t.append(" ▞▚ ", style=f"bold {th.accent}")
        t.append("O R C", style=f"bold {th.text}")
        t.append("  ▸  ", style=th.text_dim)
        t.append(scr.title_text, style=f"bold {th.accent2}")
        cur = scr.current
        if cur:
            t.append("  ▸  ", style=th.text_dim)
            status = cur.get("status", "?")
            t.append(f"{G.status_glyph(status)} {cur.get('id', '')[:44]}",
                     style=th.status_color(status))
        return t


class FlowPane(Static):
    def render(self) -> Text:
        scr = self.screen
        cur = scr.current
        return render_flow(scr.runs, self.app.tokens,
                           selected=cur.get("id") if cur else None)


class StatsStrip(Static):
    def render(self) -> Text:
        th = self.app.tokens
        cur = self.screen.current
        t = Text()
        if not cur:
            return t
        tk = cur.get("tokens") or {}
        approx = "" if tk.get("total") else "~"
        total = tk.get("total") or tk.get("estimated_total") or 0
        pieces = (
            ("brain", f"{G.BRAIN_SIGIL.get(cur.get('brain'), '◇')} {cur.get('brain', '?')}"),
            ("tokens", approx + G.fmt_tokens(total)),
            ("in/out", f"{G.fmt_tokens(tk.get('input'))}/{G.fmt_tokens(tk.get('output'))}"
             if tk.get("total") else "—"),
            ("cost", G.fmt_usd(tk.get("cost_usd")) if tk.get("cost_usd") else "—"),
            ("exit", str(cur.get("exit_code", "—"))),
        )
        for i, (label, value) in enumerate(pieces):
            if i:
                t.append("  │  ", style=th.border)
            t.append(f"{label} ", style=th.label)
            t.append(value, style=th.text)
        return t


class SessionScreen(Screen):
    CSS = """
    SessionScreen { background: $orc-bg; color: $orc-text; }
    #crumb { height: 1; }
    TabbedContent { height: 1fr; }
    #stats { height: 1; padding: 0 2; }
    #flow-scroll, #convo-scroll, #meta-scroll { padding: 1 2; }
    #session-log { padding: 0 1; }
    #log-search { dock: bottom; display: none; }
    #log-search.visible { display: block; }
    #thinking-box {
        display: none; border: round $orc-border; padding: 0 1;
        margin: 1 0; color: $orc-label;
    }
    #thinking-box.visible { display: block; }
    #prompt-box { border: round $orc-border; padding: 0 1; margin-bottom: 1; }
    #sess-ftr { height: 1; }
    """

    BINDINGS = [
        Binding("escape", "back", "back"),
        Binding("right_square_bracket", "cycle(1)", "next run", show=False),
        Binding("left_square_bracket", "cycle(-1)", "prev run", show=False),
        Binding("t", "toggle_thinking", "thinking"),
        Binding("w", "toggle_wrap", "wrap"),
        Binding("x", "kill_current", "kill"),
        Binding("slash", "search", "search", show=False),
    ]

    def __init__(self, runs: list, title: str, focus_id: str | None = None):
        super().__init__()
        self.runs = runs
        self.title_text = title
        self._idx = 0
        if focus_id:
            for i, m in enumerate(runs):
                if m.get("id") == focus_id:
                    self._idx = i
                    break
        self._confirm_kill = False
        self._tail_key = None
        self._search = ""

    @property
    def current(self) -> dict | None:
        if not self.runs:
            return None
        return self.runs[self._idx % len(self.runs)]

    # ---- layout -----------------------------------------------------------
    def compose(self) -> ComposeResult:
        yield Crumb(id="crumb")
        with TabbedContent():
            with TabPane(_spaced("flow"), id="tab-flow"):
                yield StatsStrip(id="stats")
                with VerticalScroll(id="flow-scroll"):
                    yield FlowPane(id="flow")
            with TabPane(_spaced("conversation"), id="tab-convo"):
                with VerticalScroll(id="convo-scroll"):
                    yield Static(id="prompt-box")
                    yield Static(id="thinking-box")
                    yield Markdown(id="reply-md")
            with TabPane(_spaced("log"), id="tab-log"):
                yield RichLog(id="session-log", wrap=False, highlight=False,
                              markup=False)
            with TabPane(_spaced("meta"), id="tab-meta"):
                with VerticalScroll(id="meta-scroll"):
                    yield Static(id="meta-body")
        yield Static(id="sess-ftr")
        yield Input(placeholder="search log — enter applies, esc clears",
                    id="log-search")

    def on_mount(self) -> None:
        th = self.app.tokens
        ftr = self.query_one("#sess-ftr", Static)
        t = Text(" ")
        for i, (key, label) in enumerate((
                ("esc", "back"), ("[ ]", "prev/next run"), ("tab", "panes"),
                ("t", "thinking"), ("/", "search log"), ("w", "wrap"),
                ("x", "kill"))):
            if i:
                t.append(" · ", style=th.text_dim)
            t.append(key, style=f"bold {th.accent}")
            t.append(f" {label}", style=th.text_dim)
        ftr.update(t)
        self._reload()
        self.set_interval(1.0, self._tail_log)

    # ---- data ----------------------------------------------------------------
    def _log_path(self) -> Path | None:
        cur = self.current
        if not cur:
            return None
        d = cur.get("_dir")
        if not d:
            from orc_pkg import registry
            d = registry.runs_dir() / str(cur.get("id", ""))
        return Path(d) / "output.log"

    def _reload(self) -> None:
        th = self.app.tokens
        cur = self.current
        self.query_one("#crumb", Crumb).refresh()
        self.query_one("#flow", FlowPane).refresh()
        self.query_one("#stats", StatsStrip).refresh()
        if not cur:
            return

        prompt = self.query_one("#prompt-box", Static)
        pt = Text()
        pt.append(_spaced("prompt") + f"  ({cur.get('brain', '?')} → worker)\n",
                  style=th.label)
        pt.append(str(cur.get("task", "")), style=th.text)
        prompt.update(pt)
        prompt.border_title = None

        parsed = parse_log(self._log_path()) if self._log_path() else \
            {"reply": "", "thinking": "", "plain": None}
        thinking = self.query_one("#thinking-box", Static)
        if parsed["thinking"]:
            tt = Text(_spaced("thinking") + "  (t to toggle)\n", style=th.label)
            tt.append(parsed["thinking"], style=th.text_dim)
            thinking.update(tt)
        else:
            thinking.update(Text("(no thinking recorded)", style=th.text_dim))
        body = parsed["plain"] if parsed["plain"] is not None else parsed["reply"]
        self.query_one("#reply-md", Markdown).update(
            body or "*worker has produced no output yet*")

        meta_view = {k: v for k, v in cur.items() if k != "_dir"}
        mt = Text()
        mt.append(_spaced("meta") + "\n\n", style=th.label)
        mt.append(json.dumps(meta_view, indent=2, default=str), style=th.text)
        dur = None
        if cur.get("started_at") and cur.get("ended_at"):
            from datetime import datetime
            try:
                dur = (datetime.fromisoformat(cur["ended_at"])
                       - datetime.fromisoformat(cur["started_at"])).total_seconds()
            except ValueError:
                dur = None
        if dur is not None:
            mt.append(f"\n\nduration {G.fmt_dur(dur)}", style=th.accent2)
        exit_code = cur.get("exit_code")
        if exit_code == 124:
            mt.append("   idle-timeout kill (MiniMax stall watchdog)",
                      style=th.warn)
        self.query_one("#meta-body", Static).update(mt)
        self._tail_key = None
        self._tail_log()

    def _tail_log(self) -> None:
        path = self._log_path()
        log = self.query_one("#session-log", RichLog)
        th = self.app.tokens
        if not path:
            return
        try:
            stat = path.stat()
            key = (str(path), stat.st_mtime, stat.st_size, self._search)
        except OSError:
            key = (str(path), None, None, self._search)
        if key == self._tail_key:
            return
        self._tail_key = key
        log.clear()
        try:
            lines = path.read_text(errors="replace").splitlines()
        except OSError:
            log.write(Text("(no log file)", style=th.text_dim))
            return
        if self._search:
            hits = [ln for ln in lines if self._search.lower() in ln.lower()]
            log.write(Text(f"⌕ {self._search} — {len(hits)}/{len(lines)} lines",
                           style=f"bold {th.warn}"))
            lines = hits
        for ln in lines[-400:]:
            log.write(ln)

    # ---- actions -----------------------------------------------------------
    def action_back(self) -> None:
        self.app.pop_screen()

    def action_cycle(self, delta: int) -> None:
        if self.runs:
            self._idx = (self._idx + delta) % len(self.runs)
            self._reload()

    def action_toggle_thinking(self) -> None:
        self.query_one("#thinking-box").toggle_class("visible")

    def action_toggle_wrap(self) -> None:
        log = self.query_one("#session-log", RichLog)
        log.wrap = not log.wrap
        self._tail_key = None
        self._tail_log()

    def action_search(self) -> None:
        box = self.query_one("#log-search", Input)
        box.add_class("visible")
        box.focus()

    def action_kill_current(self) -> None:
        cur = self.current
        if not cur:
            return
        if cur.get("status") not in ("running", "starting"):
            self.notify("run is not running", severity="warning")
            return
        if not self._confirm_kill:
            self._confirm_kill = True
            self.notify(f"press x again to kill {cur['id'][:32]}", timeout=3)
            return
        self._confirm_kill = False
        subprocess.run([sys.executable, "-m", "orc_pkg", "kill", cur["id"]],
                       capture_output=True)
        self.notify(f"kill sent · {cur['id'][:32]}")

    def on_input_submitted(self, event: Input.Submitted) -> None:
        if event.input.id != "log-search":
            return
        self._search = event.value.strip()
        event.input.remove_class("visible")
        self._tail_key = None
        self._tail_log()
        event.stop()

    def on_key(self, event) -> None:
        if event.key == "escape":
            box = self.query_one("#log-search", Input)
            if box.has_class("visible"):
                box.value = ""
                box.remove_class("visible")
                if self._search:
                    self._search = ""
                    self._tail_key = None
                    self._tail_log()
                event.stop()
