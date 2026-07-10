"""Run registry: one directory per delegated run, plain JSON, atomic writes."""

from __future__ import annotations

import json
import os
import re
import secrets
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path

STATUSES = ("starting", "running", "done", "failed", "killed", "orphaned")


def home() -> Path:
    """Return the orchestra home directory ($ORC_HOME or ~/.orchestra)."""
    p = os.environ.get("ORC_HOME")
    if p:
        return Path(p).expanduser()
    return Path("~/.orchestra").expanduser()


def runs_dir() -> Path:
    """Return (and create) the runs directory under home()."""
    d = home() / "runs"
    d.mkdir(parents=True, exist_ok=True)
    return d


def atomic_write_json(path, data) -> None:
    """Atomically write ``data`` as JSON to ``path`` (indent=2)."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(prefix=".tmp-", dir=str(path.parent))
    tmp_path = Path(tmp_name)
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(data, f, indent=2)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp_path, path)
    except BaseException:
        try:
            tmp_path.unlink()
        except FileNotFoundError:
            pass
        raise


def now_iso() -> str:
    """Return current UTC time as ISO-8601 string at second precision."""
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def _make_slug(task: str) -> str:
    raw = (task or "")[:24]
    slug = re.sub(r"[^a-zA-Z0-9]+", "-", raw).strip("-").lower()
    return slug or "task"


def new_run(
    task: str,
    brain: str = "human",
    cwd=None,
    provider: str = "minimax",
    model: str = "MiniMax-M3",
    session: str | None = None,
) -> Path:
    """Create a new run directory with inbox/ and meta.json, return its Path."""
    slug = _make_slug(task)
    ts = time.strftime("%Y%m%d-%H%M%S")
    run_id = f"{ts}-{slug}-{secrets.token_hex(2)}"
    run_dir = runs_dir() / run_id
    run_dir.mkdir(parents=False, exist_ok=False)
    (run_dir / "inbox").mkdir(parents=False, exist_ok=False)
    cwd_str = str(Path(cwd) if cwd else Path.cwd())
    meta = {
        "id": run_id,
        "task": task,
        "brain": brain,
        "cwd": cwd_str,
        "provider": provider,
        "model": model,
        "pid": None,
        "status": "starting",
        "started_at": now_iso(),
        "created_ts": time.time(),
        "ended_at": None,
        "exit_code": None,
        "tokens": {"estimated_total": 0},
    }
    if session:
        meta["session"] = session
    atomic_write_json(run_dir / "meta.json", meta)
    return run_dir


def read_meta(run_dir) -> dict:
    """Read and return the meta.json dict for a run directory."""
    with open(Path(run_dir) / "meta.json") as f:
        return json.load(f)


def write_meta(run_dir, meta: dict) -> None:
    """Atomically write meta.json for a run directory."""
    atomic_write_json(Path(run_dir) / "meta.json", meta)


def pid_alive(pid) -> bool:
    """Return True iff ``pid`` refers to a live process."""
    if not pid:
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def find_run(prefix: str) -> Path:
    """Find the unique run directory whose name starts with or contains prefix."""
    matches = [
        d for d in runs_dir().iterdir()
        if d.is_dir() and (d.name.startswith(prefix) or prefix in d.name)
    ]
    if len(matches) == 1:
        return matches[0]
    if not matches:
        raise SystemExit(f"orc: no runs match '{prefix}'")
    raise SystemExit(f"orc: {len(matches)} runs match '{prefix}'")


def list_runs(reconcile: bool = True) -> list:
    """Return all run meta dicts, newest first; reconcile orphans if asked."""
    items: list = []
    for run_dir in runs_dir().iterdir():
        meta_path = run_dir / "meta.json"
        if not meta_path.is_file():
            continue
        try:
            meta = read_meta(run_dir)
        except (OSError, json.JSONDecodeError):
            continue
        if reconcile and meta.get("status") in ("starting", "running"):
            pid = meta.get("pid")
            if not pid_alive(pid) and (meta.get("status") == "running" or pid is not None):
                meta["status"] = "orphaned"
                if not meta.get("ended_at"):
                    meta["ended_at"] = now_iso()
                write_meta(run_dir, meta)
        meta["_dir"] = str(run_dir)
        items.append(meta)
    items.sort(
        key=lambda m: ((m.get("created_ts") or 0), m.get("id") or ""),
        reverse=True,
    )
    return items
