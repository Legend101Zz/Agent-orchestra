"""orc top — btop-style control plane for pi-orchestra."""
import subprocess
import sys
from pathlib import Path

from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.widgets import DataTable, Footer, Header, Input, RichLog, Static

from orc_pkg import quota, registry

STATUS_STYLE = {
    "running": "bold yellow", "starting": "yellow", "done": "bold green",
    "failed": "bold red", "killed": "red", "orphaned": "dim red",
}
BRAIN_ICON = {"claude": "🧠 claude", "codex": "🤖 codex", "human": "👤 human"}


def bar(pct, width: int = 28) -> str:
    if pct is None:
        return "[dim]" + "?" * width + "[/]"
    filled = int(round(pct / 100 * width))
    color = "green" if pct > 25 else ("yellow" if pct > 10 else "red")
    return f"[{color}]{'█' * filled}[/][grey35]{'░' * (width - filled)}[/] {pct:>3}%"


class QuotaPanel(Static):
    def refresh_quota(self) -> None:
        q = quota.get_quota()
        if q["level"] == "unknown":
            self.update(f"[b]MiniMax quota[/b]  [dim]unknown — {q.get('reason', '')}[/]")
            return
        lvl_color = {"ok": "green", "warn": "yellow", "block": "red"}[q["level"]]
        self.update(
            f"[b]MiniMax quota[/b]   level: [{lvl_color}]{q['level'].upper()}[/]\n"
            f"  5-hour  {bar(q['five_hour_pct'])}   resets ~{q['window_resets_in_min']}m\n"
            f"  weekly  {bar(q['weekly_pct'])}"
        )


class OrcTop(App):
    TITLE = "orc top — pi-orchestra control plane"
    CSS = """
    #quota-panel { height: 5; padding: 0 1; border: round $accent; }
    #runs-table  { height: 1fr; border: round $accent; }
    #detail      { height: 12; border: round $accent; }
    #new-task    { dock: bottom; display: none; }
    #new-task.visible { display: block; }
    """
    BINDINGS = [
        Binding("q", "quit", "Quit"),
        Binding("k", "kill_selected", "Kill run"),
        Binding("n", "new_task", "New task"),
        Binding("r", "refresh_now", "Refresh"),
    ]

    def __init__(self):
        super().__init__()
        self._confirm_kill_id = None
        self._runs = []

    def compose(self) -> ComposeResult:
        yield Header(show_clock=True)
        yield QuotaPanel(id="quota-panel")
        yield DataTable(id="runs-table", zebra_stripes=True)
        yield RichLog(id="detail", wrap=True, highlight=False, markup=False)
        yield Input(placeholder="new task for MiniMax worker — Enter to launch, Esc to cancel",
                    id="new-task")
        yield Footer()

    def on_mount(self) -> None:
        table = self.query_one("#runs-table", DataTable)
        table.cursor_type = "row"
        table.add_columns("ID", "BRAIN", "STATUS", "STARTED", "TOK~", "TASK")
        self.refresh_data()
        self.set_interval(2.0, self.refresh_data)

    def refresh_data(self) -> None:
        self.query_one(QuotaPanel).refresh_quota()
        table = self.query_one("#runs-table", DataTable)
        selected = table.cursor_row
        table.clear()
        self._runs = registry.list_runs()
        for m in self._runs:
            style = STATUS_STYLE.get(m["status"], "")
            table.add_row(
                m["id"][:34],
                BRAIN_ICON.get(m["brain"], m["brain"]),
                f"[{style}]{m['status']}[/]" if style else m["status"],
                m["started_at"][5:19],
                str(m.get("tokens", {}).get("estimated_total", 0)),
                m["task"][:60],
                key=m["id"],
            )
        if self._runs and selected is not None and 0 <= selected < len(self._runs):
            table.move_cursor(row=selected)
        self._refresh_detail()

    def _selected_run(self):
        table = self.query_one("#runs-table", DataTable)
        if not self._runs or table.cursor_row is None:
            return None
        if 0 <= table.cursor_row < len(self._runs):
            return self._runs[table.cursor_row]
        return None

    def _refresh_detail(self) -> None:
        run = self._selected_run()
        detail = self.query_one("#detail", RichLog)
        detail.clear()
        if not run:
            return
        detail.write(f"{run['id']}  [{run['status']}]  cwd={run['cwd']}")
        log = Path(run["_dir"]) / "output.log"
        if log.exists():
            for line in log.read_text(errors="replace").splitlines()[-10:]:
                detail.write(line)

    def action_refresh_now(self) -> None:
        self.refresh_data()

    def action_kill_selected(self) -> None:
        run = self._selected_run()
        if not run:
            return
        if self._confirm_kill_id != run["id"]:
            self._confirm_kill_id = run["id"]
            self.notify(f"press k again to kill {run['id'][:24]}…", timeout=3)
            return
        self._confirm_kill_id = None
        subprocess.run([sys.executable, "-m", "orc_pkg", "kill", run["id"]],
                       capture_output=True)
        self.notify(f"kill sent to {run['id'][:24]}")
        self.refresh_data()

    def action_new_task(self) -> None:
        box = self.query_one("#new-task", Input)
        box.add_class("visible")
        box.focus()

    def on_input_submitted(self, event: Input.Submitted) -> None:
        task = event.value.strip()
        box = self.query_one("#new-task", Input)
        box.value = ""
        box.remove_class("visible")
        if task:
            subprocess.run([sys.executable, "-m", "orc_pkg", "run", task, "--bg",
                            "--brain", "human"], capture_output=True)
            self.notify("worker launched")
            self.refresh_data()

    def on_key(self, event) -> None:
        if event.key == "escape":
            box = self.query_one("#new-task", Input)
            if box.has_class("visible"):
                box.remove_class("visible")
                self.query_one("#runs-table", DataTable).focus()

    def on_data_table_row_highlighted(self, event) -> None:
        self._refresh_detail()


def run_tui() -> None:
    OrcTop().run()
