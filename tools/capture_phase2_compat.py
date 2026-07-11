#!/usr/bin/env python3
"""Capture the final Python registry/CLI compatibility oracle for Rust tests.

The generated corpus is deterministic apart from its temporary root, which is
normalized to ``<ORC_HOME>``.  Run this script while the Python implementation
still exists; after Phase 2 the checked-in output is immutable evidence.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "rust" / "crates" / "orc-core" / "tests" / "fixtures" / "python-v3"
PYTHON = [sys.executable, "-m", "orc_pkg"]
RUST = [str(ROOT / "rust" / "target" / "debug" / "orc")]


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def meta(run_id: str, **overrides: object) -> dict[str, object]:
    value: dict[str, object] = {
        "id": run_id,
        "task": f"task for {run_id}",
        "brain": "codex",
        "cwd": "/tmp/phase2-cwd",
        "provider": "minimax",
        "model": "MiniMax-M3",
        "pid": None,
        "status": "done",
        "started_at": "2026-07-11T01:02:03+00:00",
        "created_ts": 1783731723.0,
        "ended_at": "2026-07-11T01:03:04+00:00",
        "exit_code": 0,
        "tokens": {"estimated_total": 17},
    }
    value.update(overrides)
    return value


def seed(home: Path) -> None:
    runs = home / "runs"
    records = [
        meta("current", future={"kept": True}),
        {
            "id": "legacy",
            "task": "legacy missing optionals",
            "brain": "human",
            "cwd": "/tmp/legacy",
            "provider": "minimax",
            "model": "MiniMax-M3",
            "status": "done",
            "started_at": "2020-01-01T00:00:00+00:00",
            "tokens": {"estimated_total": 4},
            "legacy_unknown": 9,
        },
        meta(
            "exact-usage",
            tokens={
                "estimated_total": 2198,
                "input": 120,
                "output": 30,
                "cache_read": 2048,
                "total": 2198,
                "cost_usd": 0.000201,
                "token_future": "preserve",
            },
        ),
        meta("killed", status="killed", exit_code=-15),
        meta("orphaned", status="orphaned", exit_code=None),
        meta("rpc-agent-end", mode="rpc", tokens={"estimated_total": 12, "total": 12}),
        meta("session-linked", session="会議-session", name="指揮者 e\u0301lan"),
        meta("retry", retry_of="current"),
        meta("handoff", handoff_from="retry", attention="handoff_needed"),
        meta(
            "unicode-wide",
            task="調査 世界 e\u0301lan",
            name="奏者 🎼",
            session="セッション-界",
            cwd="/tmp/資料",
        ),
    ]
    for index, record in enumerate(records):
        record["created_ts"] = float(record.get("created_ts", 0)) + index
        run = runs / str(record["id"])
        run.mkdir(parents=True)
        (run / "inbox").mkdir()
        write_json(run / "meta.json", record)
        (run / "output.log").write_text(
            "first line\n"
            + ('{"type":"agent_end","messages":[{"usage":{"totalTokens":12}}]}\n' if record["id"] == "rpc-agent-end" else "last line 世界\n"),
            encoding="utf-8",
        )
    (runs / "corrupt").mkdir(parents=True)
    (runs / "corrupt" / "meta.json").write_text("not json\n", encoding="utf-8")
    (runs / "truncated").mkdir(parents=True)
    (runs / "truncated" / "meta.json").write_text('{"id":"truncated"', encoding="utf-8")
    write_json(
        home / "quota.json",
        {
            "five_hour_pct": 83,
            "weekly_pct": 49,
            "window_resets_in_min": 32,
            "fetched_at": 4102444800.0,
            "quota_future": "kept",
        },
    )


def normalize(value: object, home: Path) -> object:
    encoded = json.dumps(value, ensure_ascii=False).replace(str(home), "<ORC_HOME>")
    return json.loads(encoded)


def invoke(binary: list[str], args: list[str], env: dict[str, str]) -> dict[str, object]:
    result = subprocess.run(binary + args, cwd=ROOT, env=env, text=True, capture_output=True)
    stdout: object = result.stdout
    if "--json" in args or args[0] == "show":
        try:
            stdout = json.loads(result.stdout.split("\n--- output.log", 1)[0])
        except json.JSONDecodeError:
            pass
    return {"args": args, "exit": result.returncode, "stdout": stdout, "stderr": result.stderr}


def main() -> int:
    subprocess.run(["cargo", "build", "--manifest-path", str(ROOT / "rust" / "Cargo.toml"), "-q"], check=True)
    with tempfile.TemporaryDirectory(prefix="orc-phase2-oracle-") as tmp:
        home = Path(tmp) / "orchestra"
        seed(home)
        env = {**os.environ, "ORC_HOME": str(home), "HOME": str(Path(tmp) / "home")}
        commands = [
            ["list", "--json"],
            ["show", "exact-usage", "--tail", "2"],
            ["stats", "--json"],
            ["quota", "--json"],
        ]
        python = [normalize(invoke(PYTHON, args, env), home) for args in commands]
        rust = [normalize(invoke(RUST, args, env), home) for args in commands]
        raw_records = []
        for path in sorted((home / "runs").glob("*/meta.json")):
            try:
                raw_records.append(json.loads(path.read_text(encoding="utf-8")))
            except json.JSONDecodeError:
                raw_records.append({"id": path.parent.name, "corrupt_bytes": path.read_text(encoding="utf-8")})
        output = {
            "schema": 1,
            "captured_from": "live Python CLI and registry before Phase 2 deletion",
            "records": raw_records,
            "python": python,
            "rust_at_capture": rust,
        }
        FIXTURES.mkdir(parents=True, exist_ok=True)
        write_json(FIXTURES / "oracle.json", output)
        for child in home.iterdir():
            target = FIXTURES / "home" / child.name
            if child.is_dir():
                shutil.copytree(child, target, dirs_exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(child, target)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
