#!/usr/bin/env python3
"""Seed a disposable mixed-version registry for the Rust TUI VHS capture."""

from __future__ import annotations

import json
import os
import shutil
import time
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(os.environ.get("ORC_DEMO_HOME", "/tmp/orc-v3-demo"))
SESSION = "orch-v3-registry-recovery"


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n")


def stamp(offset: int) -> tuple[str, float]:
    epoch = time.time() + offset
    iso = datetime.fromtimestamp(epoch, timezone.utc).isoformat(timespec="seconds")
    return iso, epoch


def seed_run(
    run_id: str,
    task: str,
    status: str,
    offset: int,
    tokens: dict,
    **extra: object,
) -> Path:
    started_at, created_ts = stamp(offset)
    run_dir = ROOT / "runs" / run_id
    (run_dir / "inbox").mkdir(parents=True, exist_ok=True)
    meta = {
        "id": run_id,
        "task": task,
        "brain": "codex",
        "brain_model": "GPT-5 / CODEX",
        "cwd": "/Users/comreton/Desktop/pi-orchestra",
        "provider": "minimax",
        "model": "MiniMax-M3",
        "mode": "rpc" if status == "running" else "json",
        "pid": 1 if status == "running" else None,
        "status": status,
        "started_at": started_at,
        "created_ts": created_ts,
        "ended_at": None if status == "running" else started_at,
        "exit_code": None if status == "running" else (124 if status == "failed" else 0),
        "tokens": tokens,
        "session": SESSION,
        **extra,
    }
    write_json(run_dir / "meta.json", meta)
    return run_dir


def event(delta: str) -> str:
    return json.dumps(
        {
            "type": "message_update",
            "assistantMessageEvent": {"type": "text_delta", "delta": delta},
        }
    )


def main() -> None:
    shutil.rmtree(ROOT, ignore_errors=True)
    (ROOT / "runs").mkdir(parents=True)
    write_json(
        ROOT / "config.json",
        {
            "warn_pct": 25,
            "block_pct": 10,
            "cache_ttl_sec": 60,
            "max_parallel_workers": 3,
            "idle_timeout_sec": 300,
            "theme": "ember",
            "notifications": "off",
            "advisory_budget_usd": 0.5,
        },
    )
    write_json(
        ROOT / "quota.json",
        {
            "five_hour_pct": 71,
            "weekly_pct": 46,
            "window_resets_in_min": 137,
            "fetched_at": time.time(),
        },
    )
    history = []
    for index in range(96):
        history.append(
            json.dumps(
                {
                    "ts": time.time() - (95 - index) * 60,
                    "five_hour_pct": 93 - index * 0.23,
                    "weekly_pct": 52 - index * 0.06,
                }
            )
        )
    (ROOT / "quota_history.jsonl").write_text("\n".join(history) + "\n")
    write_json(
        ROOT / "sessions" / SESSION / "session.json",
        {"id": SESSION, "advisory_budget_usd": 0.5},
    )

    first = seed_run(
        "20260711-090001-map-registry-contract-a101",
        "Map every registry writer and reader, then verify the compatibility contract.",
        "done",
        -240,
        {
            "input": 421_000,
            "output": 12_400,
            "cache_read": 18_000,
            "total": 451_400,
            "estimated_total": 451_400,
            "cost_usd": 0.14,
        },
    )
    (first / "output.log").write_text(
        event("Registry inventory complete. Atomic writes and old-meta compatibility verified.\n")
        + "\n"
    )

    failed = seed_run(
        "20260711-090140-port-runner-watchdog-b202",
        "Port the JSON and RPC runner lifecycle with the idle watchdog.",
        "failed",
        -150,
        {"estimated_total": 98_000},
        attention="handoff_needed",
        failure_kind="idle_timeout",
    )
    (failed / "output.log").write_text(
        event("Runner port reached the signal-handling seam before the provider stalled.\n")
        + "\norc: idle timeout after 300s — killing worker\n"
    )

    active = seed_run(
        "20260711-090415-continue-runner-handoff-c303",
        "Continue from the verified runner checkpoint. Preserve completed registry work and finish RPC steering tests.",
        "running",
        -30,
        {"estimated_total": 37_200},
        handoff_from=failed.name,
    )
    (active / "output.log").write_text(
        event("Loaded the previous checkpoint. Registry work is intact. Implementing one-time RPC prompt delivery now.\n")
        + "\n"
        + event("Next verification: fake-pi steering, acknowledgement durability, then cross-language round trip.\n")
        + "\n"
    )
    write_json(
        active / "inbox" / "prompt-001.json",
        {"type": "prompt", "message": "Prioritize the delivery acknowledgement test."},
    )
    write_json(
        active / "inbox" / "processed" / "ack-prompt-001.json",
        {"type": "ack", "of": "prompt-001.json"},
    )


if __name__ == "__main__":
    main()

