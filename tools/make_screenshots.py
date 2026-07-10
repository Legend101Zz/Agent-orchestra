"""Regenerate docs/*.svg screenshots from a seeded demo registry.

Usage: .venv/bin/python tools/make_screenshots.py [outdir]
"""

import asyncio
import json
import os
import sys
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO))


def _write(p: Path, data: dict) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(data, indent=2))


def _evt(kind: str, delta: str) -> str:
    return json.dumps({"type": "message_update",
                       "assistantMessageEvent": {"type": kind, "delta": delta}})


def seed_demo(home: Path) -> None:
    now = time.time()

    def iso(ts):
        return time.strftime("%Y-%m-%dT%H:%M:%S+00:00", time.gmtime(ts))

    def run(rid, task, brain, status, age_s, dur_s, session=None, tokens=None,
            log_lines=()):
        rd = home / "runs" / rid
        meta = {
            "id": rid, "task": task, "brain": brain, "cwd": "/Users/demo/project",
            "provider": "minimax", "model": "MiniMax-M3",
            # a live pid keeps 'running' rows from being orphan-reconciled
            "pid": os.getpid() if status == "running" else None,
            "status": status, "started_at": iso(now - age_s),
            "created_ts": now - age_s,
            "ended_at": iso(now - age_s + dur_s) if status != "running" else None,
            "exit_code": 0 if status == "done" else (1 if status == "failed" else None),
            "tokens": tokens or {"estimated_total": 4200},
        }
        if session:
            meta["session"] = session
        _write(rd / "meta.json", meta)
        (rd / "inbox").mkdir(parents=True, exist_ok=True)
        if log_lines:
            (rd / "output.log").write_text("\n".join(log_lines) + "\n")

    sess = "orch-20260710-141201-registry-audit"
    run("20260710-141201-scan-registry-schemas-a4f2",
        "Scan every module under src/ and list all registry schema versions with file:line refs",
        "claude", "done", 3200, 190, session=sess,
        tokens={"input": 412000, "output": 9200, "cache_read": 0, "total": 421200,
                "cost_usd": 0.1346, "estimated_total": 421200},
        log_lines=[_evt("thinking_delta", "Let me scan the modules…"),
                   _evt("text_delta", "## Registry schema audit\n\n"),
                   _evt("text_delta", "| module | version | ref |\n|---|---|---|\n"),
                   _evt("text_delta", "| core.registry | v3 | src/core/registry.py:41 |\n"),
                   _evt("text_delta", "| jobs.store | v2 | src/jobs/store.py:118 |\n"),
                   json.dumps({"type": "agent_end", "messages": []})])
    run("20260710-141203-draft-migration-plan-b7c1",
        "Draft a step-by-step migration plan to unify schema v2 -> v3 (tests included)",
        "claude", "running", 2900, 0, session=sess,
        tokens={"estimated_total": 88000},
        log_lines=[_evt("text_delta", "### Migration plan\n\n1. Freeze v2 writers\n"),
                   _evt("text_delta", "2. Add dual-read shim in jobs.store\n"),
                   _evt("text_delta", "3. Backfill with `scripts/migrate_v3.py` …\n")])
    run("20260710-141206-cross-check-callers-c9d3",
        "Cross-check every caller of registry.write() against the v3 field list",
        "claude", "failed", 2800, 610, session=sess,
        tokens={"input": 96000, "output": 1400, "cache_read": 0, "total": 97400,
                "cost_usd": 0.0305, "estimated_total": 97400},
        log_lines=["worker aborted: MiniMax API stall",
                   "orc: idle timeout after 300s — killing worker"])
    run("20260710-103381-summarize-pr-feedback-e2a8",
        "Summarize review feedback from PR #482 into an action list",
        "codex", "done", 14200, 96,
        tokens={"input": 51000, "output": 2600, "cache_read": 0, "total": 53600,
                "cost_usd": 0.0184, "estimated_total": 53600},
        log_lines=[_evt("text_delta", "**Action list from PR #482**\n\n"),
                   _evt("text_delta", "- [ ] rename `flush_all` → `drain`\n"),
                   json.dumps({"type": "agent_end", "messages": []})])
    run("20260709-221540-index-docs-folder-f1b0",
        "Index the docs/ folder and produce a table of contents",
        "human", "killed", 51000, 120,
        tokens={"estimated_total": 12000},
        log_lines=["indexing docs/ …", "^C"])

    _write(home / "quota.json", {"five_hour_pct": 71, "weekly_pct": 46,
                                 "window_resets_in_min": 137, "fetched_at": now})
    hist = []
    import math
    for i in range(96):
        ts = now - (96 - i) * 900
        pct = 55 + 40 * abs(math.sin(i / 17)) - i * 0.12
        hist.append(json.dumps({"ts": ts, "five_hour_pct": round(max(8, pct)),
                                "weekly_pct": 46}))
    (home / "quota_history.jsonl").write_text("\n".join(hist) + "\n")
    _write(home / "config.json", {"theme": "ember"})


async def shoot(outdir: Path) -> None:
    from orc_pkg import metrics
    from orc_pkg.tui.app import OrcTop

    metrics.brain_usage = lambda *a, **kw: {
        "claude": {"today": {"input": 84200, "output": 31100, "cache_read": 9_800_000,
                             "cache_create": 310_000},
                   "week": {"input": 512_000, "output": 198_000, "cache_read": 0,
                            "cache_create": 0},
                   "by_model": {"claude-sonnet-5": 512_000}},
        "codex": None}

    app = OrcTop()
    async with app.run_test(size=(150, 44)) as pilot:
        await pilot.pause()
        await pilot.pause()
        (outdir / "orc-top-screenshot.svg").write_text(
            app.export_screenshot(title="orc top — pi-orchestra"))
        # drill into the session: expand, select a member, open
        await pilot.press("enter")
        await pilot.pause()
        await pilot.press("j", "enter")
        await pilot.pause()
        await pilot.pause()
        (outdir / "orc-session-screenshot.svg").write_text(
            app.export_screenshot(title="orc top — session"))
        await pilot.press("escape")
        await pilot.press("q")


def main() -> None:
    outdir = Path(sys.argv[1]) if len(sys.argv) > 1 else REPO / "docs"
    outdir.mkdir(parents=True, exist_ok=True)
    tmp = tempfile.mkdtemp(prefix="orc-demo-")
    os.environ["ORC_HOME"] = tmp
    seed_demo(Path(tmp))
    asyncio.run(shoot(outdir))
    print(f"screenshots written to {outdir}")


if __name__ == "__main__":
    main()
