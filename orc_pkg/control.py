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


def _fmt_tok(n) -> str:
    n = int(n or 0)
    if n >= 1_000_000:
        return f"{n / 1e6:.1f}M"
    if n >= 1_000:
        return f"{n / 1e3:.1f}k"
    return str(n)


def cmd_stats(args) -> int:
    from orc_pkg import metrics

    runs = registry.list_runs(reconcile=False)
    ws = metrics.worker_stats(runs)
    dv = metrics.delegated_value(runs)
    bu = metrics.brain_usage()

    if getattr(args, "json", False):
        print(json.dumps({"workers": ws, "delegated_value": dv, "brains": bu},
                         indent=2))
        return 0

    print("WORKERS (registry — exact where pi reported usage)")
    statuses = " ".join(f"{k}:{v}" for k, v in sorted(ws["by_status"].items()))
    print(f"  runs: {ws['runs']}   {statuses}")
    e = ws["exact"]
    print(f"  exact: {e['runs']} runs · in {_fmt_tok(e['input'])} / out "
          f"{_fmt_tok(e['output'])} / cache {_fmt_tok(e['cache_read'])} "
          f"· ${e['cost_usd']:.4f}")
    est = ws["estimated"]
    if est["runs"]:
        print(f"  estimated (chars/4): {est['runs']} runs · ~{_fmt_tok(est['total'])} tokens")
    for brain, b in sorted(ws["by_brain"].items()):
        print(f"    {brain:6} {b['runs']:3} runs  {_fmt_tok(b['total']):>8}  ${b['cost_usd']:.4f}")

    print("\nDELEGATED VALUE (worker tokens priced at brain API rates)")
    print(f"  saved ≈ ${dv['saved_usd']:.2f}   "
          f"({dv['multiple']}x cheaper: ${dv['brain_equiv_usd']:.2f} brain-equivalent "
          f"vs ${dv['worker_cost_usd']:.4f} MiniMax)")
    print(f"  exact basis: {dv['exact_share'] * 100:.0f}% of tokens are exact, "
          f"rest chars/4 estimates")

    print("\nBRAINS (local session logs — API-equivalent value; subscriptions are flat-rate)")
    for name in ("claude", "codex"):
        u = bu.get(name)
        if not u:
            print(f"  {name:6} n/a")
            continue
        t, w = u.get("today", {}), u.get("week", {})
        print(f"  {name:6} today in {_fmt_tok(t.get('input'))} / out {_fmt_tok(t.get('output'))}"
              f" / cache-read {_fmt_tok(t.get('cache_read'))}   "
              f"week in {_fmt_tok(w.get('input'))} / out {_fmt_tok(w.get('output'))}")
    return 0


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
