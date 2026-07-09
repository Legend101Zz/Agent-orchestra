"""Presentation & control commands: list, show, kill, quota."""
import json
import os
import signal
import sys
import time

from orc_pkg import quota, registry


QUOTA_EXIT = {"ok": 0, "warn": 2, "block": 3, "unknown": 4}
TERMINAL = ("done", "failed", "killed", "orphaned")


def cmd_list(args) -> int:
    if getattr(args, "json", False):
        print(json.dumps(registry.list_runs(), indent=2))
        return 0

    runs = registry.list_runs()
    if not runs:
        print('no runs yet — try: orc run "hello"')
        return 0

    print(f"{'ID':38} {'BRAIN':6} {'STATUS':9} {'STARTED':20} TASK")
    for r in runs:
        rid = str(r.get("id", ""))
        brain = str(r.get("brain", ""))
        status = str(r.get("status", ""))
        started_at = str(r.get("started_at", ""))
        task = str(r.get("task", ""))
        if len(task) > 48:
            task = task[:47] + "…"
        print(f"{rid[:38]:38} {brain[:6]:6} {status:9} {started_at[:19]:20} {task}")
    return 0


def cmd_show(args) -> int:
    rd = registry.find_run(args.id)
    meta = registry.read_meta(rd)
    print(json.dumps(meta, indent=2))

    output_log = rd / "output.log"
    if output_log.exists():
        print(f"\n--- output.log (last {args.tail} lines) ---")
        try:
            lines = output_log.read_text(errors="replace").splitlines()
        except OSError:
            lines = []
        for line in lines[-args.tail:]:
            print(line)
    return 0


def cmd_kill(args) -> int:
    rd = registry.find_run(args.id)
    meta = registry.read_meta(rd)

    inbox = rd / "inbox"
    inbox.mkdir(parents=True, exist_ok=True)
    kill_msg = {"type": "kill", "at": registry.now_iso()}
    registry.atomic_write_json(inbox / f"kill-{int(time.time() * 1000)}.json", kill_msg)

    pid = meta.get("pid")
    if pid and registry.pid_alive(pid):
        try:
            os.killpg(os.getpgid(pid), signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass

    for _ in range(25):
        meta = registry.read_meta(rd)
        status = meta.get("status")
        if status in TERMINAL:
            break
        if not registry.pid_alive(meta.get("pid")):
            meta["status"] = "killed"
            meta["ended_at"] = registry.now_iso()
            registry.write_meta(rd, meta)
            break
        time.sleep(0.2)

    meta = registry.read_meta(rd)
    print(f"{meta['id']}: {meta['status']}")
    return 0 if meta.get("status") in TERMINAL else 1


def cmd_quota(args) -> int:
    q = quota.get_quota(force=getattr(args, "force", False))

    if getattr(args, "json", False):
        print(json.dumps(q, indent=2))
        return QUOTA_EXIT[q["level"]]

    level = q.get("level")
    if level == "unknown":
        print(f"MiniMax quota: unknown — {q.get('reason','')}")
        return QUOTA_EXIT[level]

    print("MiniMax coding-plan quota (general):")
    print(f"  5-hour window : {q['five_hour_pct']}% remaining (resets in ~{q['window_resets_in_min']} min)")
    print(f"  weekly window : {q['weekly_pct']}% remaining")
    print(f"  level: {q['level']}   [source: {q.get('source','?')}]")
    return QUOTA_EXIT[level]
