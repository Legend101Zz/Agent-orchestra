#!/usr/bin/env python3
"""Create a disposable 500-run registry for repeatable CLI benchmarks."""

from __future__ import annotations

import json
import os
import shutil
import time
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(os.environ.get("ORC_BENCH_HOME", "/tmp/orc-v3-bench"))
RUNS = 500


def main() -> None:
    shutil.rmtree(ROOT, ignore_errors=True)
    (ROOT / "runs").mkdir(parents=True)
    now = time.time()
    for index in range(RUNS):
        created = now - index * 73
        stamp = datetime.fromtimestamp(created, timezone.utc).strftime("%Y%m%d-%H%M%S")
        run_id = f"{stamp}-benchmark-run-{index:04d}"
        run_dir = ROOT / "runs" / run_id
        (run_dir / "inbox").mkdir(parents=True)
        exact = index % 4 != 0
        tokens = (
            {
                "input": 12_000 + index,
                "output": 1_200 + index % 100,
                "cache_read": index % 900,
                "total": 13_200 + index + index % 100 + index % 900,
                "estimated_total": 13_200 + index + index % 100 + index % 900,
                "cost_usd": 0.005,
            }
            if exact
            else {"estimated_total": 18_000 + index}
        )
        meta = {
            "id": run_id,
            "task": f"Benchmark registry scan {index}",
            "brain": "codex",
            "cwd": "/tmp",
            "provider": "minimax",
            "model": "MiniMax-M3",
            "mode": "json",
            "pid": None,
            "status": "done" if index % 17 else "failed",
            "started_at": datetime.fromtimestamp(created, timezone.utc).isoformat(),
            "created_ts": created,
            "ended_at": datetime.fromtimestamp(created + 8, timezone.utc).isoformat(),
            "exit_code": 0 if index % 17 else 124,
            "tokens": tokens,
            "session": f"benchmark-session-{index // 10:02d}",
        }
        (run_dir / "meta.json").write_text(json.dumps(meta) + "\n")
        (run_dir / "output.log").write_text("benchmark output\n")

    quota = {
        "five_hour_pct": 70,
        "weekly_pct": 45,
        "window_resets_in_min": 120,
        "fetched_at": now,
    }
    (ROOT / "quota.json").write_text(json.dumps(quota) + "\n")
    print(f"seeded {RUNS} runs in {ROOT}")


if __name__ == "__main__":
    main()
