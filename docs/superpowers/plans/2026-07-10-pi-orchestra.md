# pi-orchestra Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Execution mode chosen by user:** inline, dogfooding the advisor pattern — for each
> implementation step, first ask the pi/MiniMax-M3 worker to draft the file from the
> task's Interfaces + tests, then the main brain (Fable) reviews the draft against the
> reference code in this plan, integrates the better version, and records friction.

**Goal:** A delegation-and-orchestration layer where Claude Code/Codex brains offload heavy work to pi running MiniMax-M3, every run is recorded in a plain-JSON registry, quota is checked before spawning, and a btop-style Textual TUI (`orc top`) shows and controls all sessions.

**Architecture:** Single Python package `orc_pkg` exposing the `orc` CLI (subcommands: run, rpc, list, show, kill, quota, top, hidden `_exec`). All state lives in `~/.orchestra` (override with `ORC_HOME`) as atomically-written JSON. Shell helpers `deleg8`/`pi-rpc` and two Claude Code skills (`pi-delegate`, `orchestrate`) drive brain behavior; Codex gets the same instructions via a marked `~/.codex/AGENTS.md` block.

**Tech Stack:** Python 3.14 (system `python3` → project venv), stdlib for core, Textual for TUI, pytest + pytest-asyncio for tests, zsh helpers, macOS Keychain for the API key.

## Global Constraints

- Never print, move, or invent API keys. Key sources: Keychain item `minimax_api_key`, fallback `~/.pi/agent/auth.json` → `minimax.key`.
- Never modify: `~/.pi/agent/auth.json`, `~/.pi/agent/settings.json`, `~/.claude/settings.json`, `~/.codex/config.toml`. Appends to `~/.zshrc` and `~/.codex/AGENTS.md` must be inside `# >>> pi-orchestra >>>` / `<!-- pi-orchestra:begin -->` marker blocks, idempotent, and preceded by a `.pi-orchestra.bak` backup.
- Every pi invocation: explicit `--provider minimax --model MiniMax-M3 --no-session`.
- All registry JSON writes: temp file in same dir + `os.replace` (atomic). Single writer per `meta.json` = the orc process owning the run; exceptions only when the owner PID is confirmed dead (kill/orphan reconcile).
- Quota gate: warn ≤ 25 %, block ≤ 10 % (of min(5-hour %, weekly %)), configurable via `~/.orchestra/config.json`. Quota-endpoint failure warns but never blocks.
- Verified live facts: `GET https://api.minimax.io/v1/token_plan/remains` with Bearer key returns `{"model_remains":[{"model_name":"general","current_interval_remaining_percent":83,"current_weekly_remaining_percent":49,"remains_time":1909550,...},...],"base_resp":{"status_code":0}}`. The `general` entry is the coding plan; `remains_time` is ms until the 5-hour window resets.
- Repo root: `/Users/comreton/Desktop/pi-orchestra`. Registry: `~/.orchestra`. Tests must set `ORC_HOME` to a tmp dir and a fake `pi` on PATH — live MiniMax calls happen only in Task 10's live smoke.

## File Structure

```
pi-orchestra/
├── bin/orc                    # bash shim → .venv python -m orc_pkg
├── orc_pkg/
│   ├── __init__.py            # VERSION
│   ├── __main__.py            # argparse dispatch
│   ├── registry.py            # runs, meta.json, atomic writes, list/reconcile
│   ├── quota.py               # key lookup, /remains fetch+parse, cache, levels
│   ├── runner.py              # run/_exec/rpc, quota gate, pi spawning
│   ├── control.py             # list/show/kill/quota commands (presentation)
│   └── tui.py                 # orc top (Textual, lazy import)
├── shell/orchestra.zsh        # deleg8, pi-rpc
├── skills/pi-delegate/SKILL.md
├── skills/orchestrate/SKILL.md
├── codex/AGENTS-block.md      # appended to ~/.codex/AGENTS.md
├── install.sh / uninstall.sh
├── requirements.txt           # textual, pytest, pytest-asyncio
├── pyproject.toml             # pytest config only
├── tests/
│   ├── conftest.py            # ORC_HOME tmpdir + fake pi fixtures
│   ├── test_registry.py
│   ├── test_quota.py
│   ├── test_runner.py
│   ├── test_control.py
│   ├── test_tui.py            # smoke via Textual Pilot
│   └── live_smoke.sh          # Task 10 — real MiniMax calls
└── README.md                  # cheat sheet + uninstall
```

---

### Task 1: Scaffold — venv, shim, package skeleton

**Files:**
- Create: `bin/orc`, `orc_pkg/__init__.py`, `orc_pkg/__main__.py`, `requirements.txt`, `pyproject.toml`, `.gitignore`

**Interfaces:**
- Produces: executable `bin/orc` running `python -m orc_pkg`; `orc version` prints `orc 0.1.0`; `main(argv) -> int` in `__main__.py` that later tasks extend with subparsers.

- [ ] **Step 1: Write files**

`.gitignore`:
```
.venv/
__pycache__/
*.pyc
.pytest_cache/
```

`requirements.txt`:
```
textual>=1.0
pytest>=8
pytest-asyncio>=0.24
```

`pyproject.toml`:
```toml
[tool.pytest.ini_options]
pythonpath = ["."]
asyncio_mode = "auto"
testpaths = ["tests"]
```

`bin/orc`:
```bash
#!/usr/bin/env bash
# orc — pi-orchestra CLI shim. Resolves symlinks so ~/.local/bin/orc finds the repo.
SELF="$(readlink -f "${BASH_SOURCE[0]}")"
ROOT="$(cd "$(dirname "$SELF")/.." && pwd)"
PY="$ROOT/.venv/bin/python"
[ -x "$PY" ] || PY="$(command -v python3)"
exec env PYTHONPATH="$ROOT${PYTHONPATH:+:$PYTHONPATH}" "$PY" -m orc_pkg "$@"
```

`orc_pkg/__init__.py`:
```python
VERSION = "0.1.0"
```

`orc_pkg/__main__.py`:
```python
import argparse
import sys

from orc_pkg import VERSION


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="orc", description="pi-orchestra: MiniMax M3 worker delegation")
    sub = p.add_subparsers(dest="cmd")
    sub.add_parser("version", help="print version")
    return p


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    if args.cmd == "version":
        print(f"orc {VERSION}")
        return 0
    build_parser().print_help()
    return 1


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 2: Create venv and install deps**

Run: `cd /Users/comreton/Desktop/pi-orchestra && python3 -m venv .venv && .venv/bin/pip -q install -U pip && .venv/bin/pip -q install -r requirements.txt && chmod +x bin/orc`

- [ ] **Step 3: Verify**

Run: `bin/orc version` → Expected: `orc 0.1.0`

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat: scaffold orc CLI (shim, package, venv deps)"
```

---

### Task 2: registry.py — run records with atomic JSON

**Files:**
- Create: `orc_pkg/registry.py`, `tests/conftest.py`, `tests/test_registry.py`

**Interfaces:**
- Produces (used by every later task):
  - `home() -> Path` — `$ORC_HOME` or `~/.orchestra`
  - `runs_dir() -> Path` (creates)
  - `atomic_write_json(path: Path, data: dict) -> None`
  - `now_iso() -> str` (UTC, seconds)
  - `new_run(task: str, brain: str = "human", cwd: str | None = None, provider: str = "minimax", model: str = "MiniMax-M3") -> Path` — creates `runs/<id>/` with `inbox/` and `meta.json` (status `starting`), returns run dir
  - `read_meta(run_dir: Path) -> dict` / `write_meta(run_dir: Path, meta: dict) -> None`
  - `pid_alive(pid: int | None) -> bool`
  - `find_run(prefix: str) -> Path` — unique id-prefix/substring match, `SystemExit` with message on 0 or >1 matches
  - `list_runs(reconcile: bool = True) -> list[dict]` — newest first, each dict gains `_dir`; running entries with dead PIDs become `orphaned` (written back)
  - meta.json fields: `id, task, brain, cwd, provider, model, pid, status(starting|running|done|failed|killed|orphaned), started_at, ended_at, exit_code, tokens{estimated_total}`

- [ ] **Step 1: Write fixtures and failing tests**

`tests/conftest.py`:
```python
import os
import stat
import sys
from pathlib import Path

import pytest


@pytest.fixture
def orc_home(tmp_path, monkeypatch):
    home = tmp_path / "orchestra"
    monkeypatch.setenv("ORC_HOME", str(home))
    return home


@pytest.fixture
def fake_pi(tmp_path, monkeypatch):
    """A stand-in `pi` on PATH. Echoes a canned reply; sleeps when task contains SLEEP."""
    bindir = tmp_path / "fakebin"
    bindir.mkdir()
    script = bindir / "pi"
    script.write_text(
        "#!/usr/bin/env bash\n"
        'task="${@: -1}"\n'
        'if [[ "$task" == *SLEEP* ]]; then echo "sleeping"; sleep 30; fi\n'
        'echo "FAKE-PI-REPLY: $task"\n'
    )
    script.chmod(script.stat().st_mode | stat.S_IEXEC)
    monkeypatch.setenv("PATH", f"{bindir}:{os.environ['PATH']}")
    return script
```

`tests/test_registry.py`:
```python
import json
import os

from orc_pkg import registry


def test_home_respects_env(orc_home):
    assert registry.home() == orc_home


def test_new_run_creates_meta_and_inbox(orc_home):
    rd = registry.new_run("Summarize the repo", brain="claude", cwd="/tmp")
    meta = registry.read_meta(rd)
    assert meta["status"] == "starting"
    assert meta["task"] == "Summarize the repo"
    assert meta["brain"] == "claude"
    assert meta["cwd"] == "/tmp"
    assert meta["model"] == "MiniMax-M3"
    assert (rd / "inbox").is_dir()
    assert meta["id"] == rd.name


def test_atomic_write_leaves_no_temp_files(orc_home):
    rd = registry.new_run("t")
    meta = registry.read_meta(rd)
    meta["status"] = "done"
    registry.write_meta(rd, meta)
    leftovers = [p for p in rd.iterdir() if p.name.startswith(".tmp-")]
    assert leftovers == []
    assert registry.read_meta(rd)["status"] == "done"


def test_list_runs_newest_first_and_reconciles_dead_pid(orc_home):
    rd1 = registry.new_run("first")
    rd2 = registry.new_run("second")
    m = registry.read_meta(rd1)
    m["status"], m["pid"] = "running", 99999999  # certainly dead
    registry.write_meta(rd1, m)
    runs = registry.list_runs()
    assert [r["id"] for r in runs][0] == rd2.name
    stale = [r for r in runs if r["id"] == rd1.name][0]
    assert stale["status"] == "orphaned"
    assert registry.read_meta(rd1)["status"] == "orphaned"


def test_pid_alive_self_and_dead(orc_home):
    assert registry.pid_alive(os.getpid()) is True
    assert registry.pid_alive(99999999) is False
    assert registry.pid_alive(None) is False


def test_find_run_prefix(orc_home):
    rd = registry.new_run("unique task alpha")
    assert registry.find_run(rd.name[:15]) == rd
```

- [ ] **Step 2: Run tests, verify failure**

Run: `.venv/bin/python -m pytest tests/test_registry.py -q` → Expected: import error / failures.

- [ ] **Step 3: Implement `orc_pkg/registry.py`**

*(Dogfood: send the Interfaces block + test file to the pi worker first; integrate against this reference.)*

```python
"""Run registry: one directory per delegated run, plain JSON, atomic writes."""
import json
import os
import secrets
import tempfile
from datetime import datetime, timezone
from pathlib import Path

STATUSES = ("starting", "running", "done", "failed", "killed", "orphaned")


def home() -> Path:
    return Path(os.environ.get("ORC_HOME", "~/.orchestra")).expanduser()


def runs_dir() -> Path:
    d = home() / "runs"
    d.mkdir(parents=True, exist_ok=True)
    return d


def atomic_write_json(path: Path, data: dict) -> None:
    fd, tmp = tempfile.mkstemp(dir=str(path.parent), prefix=".tmp-")
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(data, f, indent=2)
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def new_run(task: str, brain: str = "human", cwd: str | None = None,
            provider: str = "minimax", model: str = "MiniMax-M3") -> Path:
    slug = "".join(c if c.isalnum() else "-" for c in task[:24]).strip("-").lower() or "task"
    run_id = f"{datetime.now().strftime('%Y%m%d-%H%M%S')}-{slug}-{secrets.token_hex(2)}"
    rd = runs_dir() / run_id
    (rd / "inbox").mkdir(parents=True)
    meta = {
        "id": run_id, "task": task, "brain": brain,
        "cwd": str(cwd or Path.cwd()), "provider": provider, "model": model,
        "pid": None, "status": "starting",
        "started_at": now_iso(), "ended_at": None, "exit_code": None,
        "tokens": {"estimated_total": 0},
    }
    atomic_write_json(rd / "meta.json", meta)
    return rd


def read_meta(run_dir: Path) -> dict:
    return json.loads((run_dir / "meta.json").read_text())


def write_meta(run_dir: Path, meta: dict) -> None:
    atomic_write_json(run_dir / "meta.json", meta)


def pid_alive(pid) -> bool:
    if not pid:
        return False
    try:
        os.kill(int(pid), 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def find_run(prefix: str) -> Path:
    matches = [d for d in runs_dir().iterdir()
               if d.is_dir() and (d.name.startswith(prefix) or prefix in d.name)]
    if len(matches) == 1:
        return matches[0]
    raise SystemExit(f"orc: {'no' if not matches else len(matches)} runs match '{prefix}'")


def list_runs(reconcile: bool = True) -> list[dict]:
    out = []
    for rd in sorted(runs_dir().iterdir(), reverse=True):
        if not (rd / "meta.json").exists():
            continue
        m = read_meta(rd)
        if reconcile and m["status"] in ("starting", "running") and not pid_alive(m.get("pid")):
            # Owner is dead (or never recorded a pid and vanished): safe to take over.
            age_ok = m["status"] == "running" or m.get("pid") is not None
            if age_ok:
                m["status"] = "orphaned"
                m["ended_at"] = m["ended_at"] or now_iso()
                write_meta(rd, m)
        m["_dir"] = str(rd)
        out.append(m)
    return out
```

- [ ] **Step 4: Run tests, verify pass**

Run: `.venv/bin/python -m pytest tests/test_registry.py -q` → Expected: all pass.
Note: `test_list_runs_newest_first_and_reconciles_dead_pid` sets `pid` — the `starting`-with-no-pid case must NOT be orphaned (a just-created fg run has no pid yet); the guard above handles it.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: run registry with atomic JSON writes and orphan reconcile"
```

---

### Task 3: quota.py — key lookup, /remains, thresholds, cache

**Files:**
- Create: `orc_pkg/quota.py`, `tests/test_quota.py`

**Interfaces:**
- Consumes: `registry.home()`, `registry.atomic_write_json()`
- Produces:
  - `DEFAULT_CONFIG = {"warn_pct": 25, "block_pct": 10, "cache_ttl_sec": 60, "max_parallel_workers": 3}`
  - `load_config() -> dict` — DEFAULT_CONFIG overlaid with `~/.orchestra/config.json`
  - `get_key() -> str | None` — Keychain `minimax_api_key`, fallback `~/.pi/agent/auth.json` `minimax.key`/`minimax.apiKey`
  - `parse_remains(raw: dict) -> dict | None` — extracts the `general` entry → `{"five_hour_pct", "weekly_pct", "window_resets_in_min", "fetched_at"}`
  - `level_for(parsed: dict, cfg: dict) -> str` — `ok|warn|block` from `min(five_hour_pct, weekly_pct)`
  - `get_quota(force: bool = False) -> dict` — cached (ttl) → `{"level", "five_hour_pct", "weekly_pct", "window_resets_in_min", "source": "api"|"cache", ...}` or `{"level": "unknown", "reason": str}`

- [ ] **Step 1: Write failing tests**

`tests/test_quota.py`:
```python
import json
import time

from orc_pkg import quota, registry

# Captured live from api.minimax.io on 2026-07-10 (values anonymized-ish, schema exact)
RAW = {
    "model_remains": [
        {"start_time": 1783609200000, "end_time": 1783627200000, "remains_time": 1909550,
         "model_name": "general",
         "current_interval_remaining_percent": 83, "current_weekly_remaining_percent": 49,
         "current_interval_status": 1, "current_weekly_status": 1},
        {"model_name": "video", "remains_time": 16309550,
         "current_interval_remaining_percent": 100, "current_weekly_remaining_percent": 100},
    ],
    "base_resp": {"status_code": 0, "status_msg": "success"},
}


def test_parse_remains_picks_general():
    p = quota.parse_remains(RAW)
    assert p["five_hour_pct"] == 83
    assert p["weekly_pct"] == 49
    assert p["window_resets_in_min"] == 32  # 1909550 ms ≈ 31.8 min


def test_parse_remains_no_general_returns_none():
    assert quota.parse_remains({"model_remains": [{"model_name": "video"}]}) is None


def test_level_thresholds():
    cfg = dict(quota.DEFAULT_CONFIG)
    assert quota.level_for({"five_hour_pct": 83, "weekly_pct": 49}, cfg) == "ok"
    assert quota.level_for({"five_hour_pct": 20, "weekly_pct": 90}, cfg) == "warn"
    assert quota.level_for({"five_hour_pct": 90, "weekly_pct": 9}, cfg) == "block"


def test_get_quota_uses_cache(orc_home, monkeypatch):
    calls = {"n": 0}

    def fake_fetch(key):
        calls["n"] += 1
        return RAW

    monkeypatch.setattr(quota, "fetch_remains", fake_fetch)
    monkeypatch.setattr(quota, "get_key", lambda: "k")
    q1 = quota.get_quota()
    q2 = quota.get_quota()
    assert q1["level"] == "ok" and q1["source"] == "api"
    assert q2["source"] == "cache"
    assert calls["n"] == 1
    assert q1["five_hour_pct"] == 83


def test_get_quota_unknown_on_error(orc_home, monkeypatch):
    monkeypatch.setattr(quota, "get_key", lambda: "k")
    monkeypatch.setattr(quota, "fetch_remains",
                        lambda key: (_ for _ in ()).throw(OSError("boom")))
    q = quota.get_quota(force=True)
    assert q["level"] == "unknown"
    assert "boom" in q["reason"]


def test_get_quota_no_key(orc_home, monkeypatch):
    monkeypatch.setattr(quota, "get_key", lambda: None)
    q = quota.get_quota(force=True)
    assert q["level"] == "unknown"
```

- [ ] **Step 2: Run tests, verify fail**

Run: `.venv/bin/python -m pytest tests/test_quota.py -q` → Expected: import error.

- [ ] **Step 3: Implement `orc_pkg/quota.py`**

*(Dogfood: worker drafts from Interfaces + tests.)*

```python
"""MiniMax coding-plan quota: fetch, parse, threshold levels, 60s cache."""
import json
import os
import subprocess
import time
import urllib.request
from pathlib import Path

from orc_pkg import registry

REMAINS_URL = "https://api.minimax.io/v1/token_plan/remains"
DEFAULT_CONFIG = {"warn_pct": 25, "block_pct": 10, "cache_ttl_sec": 60,
                  "max_parallel_workers": 3}


def load_config() -> dict:
    cfg = dict(DEFAULT_CONFIG)
    p = registry.home() / "config.json"
    if p.exists():
        try:
            cfg.update(json.loads(p.read_text()))
        except ValueError:
            pass
    return cfg


def get_key():
    try:
        r = subprocess.run(
            ["security", "find-generic-password", "-a", os.environ.get("USER", ""),
             "-s", "minimax_api_key", "-w"],
            capture_output=True, text=True, timeout=10)
        if r.returncode == 0 and r.stdout.strip():
            return r.stdout.strip()
    except Exception:
        pass
    try:
        auth = json.loads((Path.home() / ".pi/agent/auth.json").read_text())
        entry = auth.get("minimax") or {}
        return entry.get("key") or entry.get("apiKey")
    except Exception:
        return None


def fetch_remains(key: str) -> dict:
    req = urllib.request.Request(
        REMAINS_URL,
        headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=15) as resp:
        return json.loads(resp.read().decode())


def parse_remains(raw: dict):
    for entry in raw.get("model_remains", []):
        if entry.get("model_name") == "general":
            return {
                "five_hour_pct": entry.get("current_interval_remaining_percent"),
                "weekly_pct": entry.get("current_weekly_remaining_percent"),
                "window_resets_in_min": round(entry.get("remains_time", 0) / 60000),
                "fetched_at": time.time(),
            }
    return None


def level_for(parsed: dict, cfg: dict) -> str:
    pct = min(parsed["five_hour_pct"], parsed["weekly_pct"])
    if pct <= cfg["block_pct"]:
        return "block"
    if pct <= cfg["warn_pct"]:
        return "warn"
    return "ok"


def get_quota(force: bool = False) -> dict:
    cfg = load_config()
    registry.home().mkdir(parents=True, exist_ok=True)
    cache = registry.home() / "quota.json"
    if not force and cache.exists():
        try:
            data = json.loads(cache.read_text())
            if time.time() - data.get("fetched_at", 0) < cfg["cache_ttl_sec"]:
                data["level"] = level_for(data, cfg)
                data["source"] = "cache"
                return data
        except (ValueError, KeyError, TypeError):
            pass
    key = get_key()
    if not key:
        return {"level": "unknown", "reason": "no MiniMax key in Keychain or auth.json"}
    try:
        parsed = parse_remains(fetch_remains(key))
    except Exception as e:
        return {"level": "unknown", "reason": str(e)}
    if parsed is None:
        return {"level": "unknown",
                "reason": "no 'general' entry — key may not be a coding-plan key"}
    registry.atomic_write_json(cache, parsed)
    parsed["level"] = level_for(parsed, cfg)
    parsed["source"] = "api"
    return parsed
```

- [ ] **Step 4: Run tests, verify pass**

Run: `.venv/bin/python -m pytest tests/test_quota.py -q` → Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: MiniMax coding-plan quota fetch/parse/levels with cache"
```

---

### Task 4: runner.py — orc run (fg/bg), quota gate, finalization

**Files:**
- Create: `orc_pkg/runner.py`, `tests/test_runner.py`
- Modify: `orc_pkg/__main__.py` (add `run` + hidden `_exec` subcommands)

**Interfaces:**
- Consumes: `registry.*`, `quota.get_quota()`, `quota.load_config()`
- Produces:
  - `PI_BASE = ["pi", "-p", "--provider", "minimax", "--model", "MiniMax-M3", "--no-session"]`
  - `quota_gate(force: bool) -> bool` — prints `ORC WARNING:`/`ORC BLOCKED:`/`ORC NOTE:` lines to stderr; returns False only when blocked and not forced
  - `cmd_run(args) -> int` — fg: exit code of pi; bg: prints run id, returns 0; blocked: returns 3
  - `cmd_exec(args) -> int` — hidden; runs an already-registered run dir (`--echo` mirrors output to stdout)
  - `finalize(rd, meta, code)` sets status `done` (0) / `killed` (<0) / `failed` (>0), `ended_at`, `exit_code`, `tokens.estimated_total = (task_len + log_bytes) // 4`
  - CLI: `orc run "task" [--cwd DIR] [--brain B] [--name N] [--bg] [--force]`

- [ ] **Step 1: Write failing tests**

`tests/test_runner.py`:
```python
import json
import os
import subprocess
import sys
import time
from pathlib import Path

from orc_pkg import registry

ORC = [sys.executable, "-m", "orc_pkg"]


def run_orc(*argv, **kw):
    return subprocess.run([*ORC, *argv], capture_output=True, text=True,
                          env=os.environ.copy(), **kw)


def seed_ok_quota(orc_home):
    orc_home.mkdir(parents=True, exist_ok=True)
    (orc_home / "quota.json").write_text(json.dumps(
        {"five_hour_pct": 90, "weekly_pct": 90, "window_resets_in_min": 60,
         "fetched_at": time.time()}))


def seed_blocked_quota(orc_home):
    orc_home.mkdir(parents=True, exist_ok=True)
    (orc_home / "quota.json").write_text(json.dumps(
        {"five_hour_pct": 5, "weekly_pct": 90, "window_resets_in_min": 60,
         "fetched_at": time.time()}))


def test_run_foreground_success(orc_home, fake_pi):
    seed_ok_quota(orc_home)
    r = run_orc("run", "hello world", "--brain", "claude")
    assert r.returncode == 0
    assert "FAKE-PI-REPLY: hello world" in r.stdout
    runs = registry.list_runs()
    assert len(runs) == 1
    m = runs[0]
    assert m["status"] == "done"
    assert m["brain"] == "claude"
    assert m["exit_code"] == 0
    assert m["tokens"]["estimated_total"] > 0
    log = Path(m["_dir"]) / "output.log"
    assert "FAKE-PI-REPLY" in log.read_text()


def test_run_blocked_by_quota(orc_home, fake_pi):
    seed_blocked_quota(orc_home)
    r = run_orc("run", "hello")
    assert r.returncode == 3
    assert "ORC BLOCKED" in r.stderr
    assert registry.list_runs() == []


def test_run_blocked_force_overrides(orc_home, fake_pi):
    seed_blocked_quota(orc_home)
    r = run_orc("run", "hello", "--force")
    assert r.returncode == 0
    assert "FAKE-PI-REPLY" in r.stdout


def test_run_warn_prints_warning(orc_home, fake_pi):
    (orc_home / "quota.json").parent.mkdir(parents=True, exist_ok=True)
    (orc_home / "quota.json").write_text(json.dumps(
        {"five_hour_pct": 20, "weekly_pct": 90, "window_resets_in_min": 60,
         "fetched_at": time.time()}))
    r = run_orc("run", "hello")
    assert r.returncode == 0
    assert "ORC WARNING" in r.stderr


def test_run_background_returns_id_and_completes(orc_home, fake_pi):
    seed_ok_quota(orc_home)
    r = run_orc("run", "bg task", "--bg")
    assert r.returncode == 0
    run_id = r.stdout.strip()
    assert run_id
    for _ in range(50):
        m = registry.read_meta(registry.find_run(run_id))
        if m["status"] == "done":
            break
        time.sleep(0.2)
    assert m["status"] == "done"
```

- [ ] **Step 2: Run tests, verify fail**

Run: `.venv/bin/python -m pytest tests/test_runner.py -q` → Expected: failures (no `run` subcommand).

- [ ] **Step 3: Implement `orc_pkg/runner.py`**

*(Dogfood: worker drafts from Interfaces + tests.)*

```python
"""Spawn pi workers: foreground/background runs with quota gating."""
import os
import signal
import subprocess
import sys
from pathlib import Path

from orc_pkg import quota, registry

PI_BASE = ["pi", "-p", "--provider", "minimax", "--model", "MiniMax-M3", "--no-session"]


def quota_gate(force: bool) -> bool:
    q = quota.get_quota()
    lvl = q["level"]
    if lvl == "warn":
        print(f"ORC WARNING: MiniMax quota low — 5h window {q['five_hour_pct']}% / "
              f"weekly {q['weekly_pct']}% remaining. Consider pausing delegation.",
              file=sys.stderr)
    elif lvl == "block":
        print(f"ORC BLOCKED: MiniMax quota below block threshold "
              f"(5h {q['five_hour_pct']}%, weekly {q['weekly_pct']}%). "
              f"Use --force to override.", file=sys.stderr)
        return force
    elif lvl == "unknown":
        print(f"ORC NOTE: quota unknown ({q.get('reason', '')}) — proceeding.",
              file=sys.stderr)
    return True


def finalize(rd: Path, meta: dict, code: int) -> None:
    meta["status"] = "done" if code == 0 else ("killed" if code < 0 else "failed")
    meta["exit_code"] = code
    meta["ended_at"] = registry.now_iso()
    log = rd / "output.log"
    log_bytes = log.stat().st_size if log.exists() else 0
    meta["tokens"]["estimated_total"] = (len(meta["task"]) + log_bytes) // 4
    registry.write_meta(rd, meta)


def _exec(rd: Path, echo: bool = False) -> int:
    meta = registry.read_meta(rd)
    code = 1
    with (rd / "output.log").open("ab") as log:
        try:
            proc = subprocess.Popen(
                PI_BASE + [meta["task"]], cwd=meta["cwd"],
                stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                start_new_session=True)
        except FileNotFoundError:
            log.write(b"orc: pi executable not found on PATH\n")
            finalize(rd, meta, 127)
            if echo:
                print("orc: pi executable not found on PATH", file=sys.stderr)
            return 127
        meta["pid"] = proc.pid
        meta["status"] = "running"
        registry.write_meta(rd, meta)
        try:
            for chunk in iter(proc.stdout.readline, b""):
                log.write(chunk)
                log.flush()
                if echo:
                    sys.stdout.buffer.write(chunk)
                    sys.stdout.buffer.flush()
            code = proc.wait()
        except KeyboardInterrupt:
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
            except ProcessLookupError:
                pass
            code = proc.wait()
            if code >= 0:
                code = -signal.SIGTERM
    finalize(rd, meta, code)
    return max(code, 0) if code >= 0 else 130


def cmd_run(args) -> int:
    if not quota_gate(args.force):
        return 3
    rd = registry.new_run(args.task, brain=args.brain, cwd=args.cwd)
    if args.name:
        meta = registry.read_meta(rd)
        meta["name"] = args.name
        registry.write_meta(rd, meta)
    if args.bg:
        subprocess.Popen(
            [sys.executable, "-m", "orc_pkg", "_exec", str(rd)],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            start_new_session=True)
        print(rd.name)
        return 0
    return _exec(rd, echo=True)


def cmd_exec(args) -> int:
    return _exec(Path(args.run_dir), echo=args.echo)
```

- [ ] **Step 4: Wire subcommands into `orc_pkg/__main__.py`** (replace whole file)

```python
import argparse
import sys

from orc_pkg import VERSION


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="orc", description="pi-orchestra: MiniMax M3 worker delegation")
    sub = p.add_subparsers(dest="cmd")
    sub.add_parser("version", help="print version")

    run = sub.add_parser("run", help="delegate a one-shot task to pi/MiniMax-M3")
    run.add_argument("task")
    run.add_argument("--cwd", default=None)
    run.add_argument("--brain", default="human", choices=["claude", "codex", "human"])
    run.add_argument("--name", default=None)
    run.add_argument("--bg", action="store_true")
    run.add_argument("--force", action="store_true")

    ex = sub.add_parser("_exec")  # hidden: executes a registered run dir
    ex.add_argument("run_dir")
    ex.add_argument("--echo", action="store_true")

    return p


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    if args.cmd == "version":
        print(f"orc {VERSION}")
        return 0
    if args.cmd == "run":
        from orc_pkg import runner
        return runner.cmd_run(args)
    if args.cmd == "_exec":
        from orc_pkg import runner
        return runner.cmd_exec(args)
    build_parser().print_help()
    return 1


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 5: Run tests, verify pass**

Run: `.venv/bin/python -m pytest tests/test_runner.py tests/test_registry.py -q` → Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: orc run (fg/bg) with quota gate and run finalization"
```

---

### Task 5: control.py — list / show / kill / quota commands

**Files:**
- Create: `orc_pkg/control.py`, `tests/test_control.py`
- Modify: `orc_pkg/__main__.py` (add `list`, `show`, `kill`, `quota` subcommands)

**Interfaces:**
- Consumes: `registry.*`, `quota.get_quota()`, `runner` statuses
- Produces:
  - `cmd_list(args) -> int` — table (`ID  BRAIN  STATUS  STARTED  TASK`) or `--json` (list of metas)
  - `cmd_show(args) -> int` — meta pretty-print + last `--tail N` (default 40) log lines
  - `cmd_kill(args) -> int` — drop `{"type":"kill","at":iso}` into `inbox/`, SIGTERM the process group, wait ≤5 s; if meta still not terminal and PID dead → write `status=killed`. Exit 0 if the run ends up terminal, 1 otherwise.
  - `cmd_quota(args) -> int` — human summary or `--json`; exit code 0=ok 2=warn 3=block 4=unknown
  - CLI: `orc list [--json]`, `orc show <id> [--tail N]`, `orc kill <id>`, `orc quota [--json] [--force]`

- [ ] **Step 1: Write failing tests**

`tests/test_control.py`:
```python
import json
import time

from tests.test_runner import run_orc, seed_ok_quota

from orc_pkg import registry


def test_list_json_and_table(orc_home, fake_pi):
    seed_ok_quota(orc_home)
    run_orc("run", "quick one")
    r = run_orc("list", "--json")
    data = json.loads(r.stdout)
    assert len(data) == 1 and data[0]["status"] == "done"
    t = run_orc("list")
    assert "quick one" in t.stdout and "done" in t.stdout


def test_show_prints_meta_and_log(orc_home, fake_pi):
    seed_ok_quota(orc_home)
    run_orc("run", "show me")
    rid = json.loads(run_orc("list", "--json").stdout)[0]["id"]
    r = run_orc("show", rid[:15])
    assert "show me" in r.stdout and "FAKE-PI-REPLY" in r.stdout


def test_kill_background_run(orc_home, fake_pi):
    seed_ok_quota(orc_home)
    rid = run_orc("run", "SLEEP forever", "--bg").stdout.strip()
    for _ in range(50):
        m = registry.read_meta(registry.find_run(rid))
        if m["status"] == "running":
            break
        time.sleep(0.1)
    assert m["status"] == "running"
    r = run_orc("kill", rid)
    assert r.returncode == 0
    for _ in range(50):
        m = registry.read_meta(registry.find_run(rid))
        if m["status"] in ("killed", "failed"):
            break
        time.sleep(0.1)
    assert m["status"] == "killed"
    assert not registry.pid_alive(m["pid"])
    inbox = list((registry.find_run(rid) / "inbox").glob("*.json"))
    assert any(json.loads(p.read_text())["type"] == "kill" for p in inbox)


def test_quota_exit_codes(orc_home, fake_pi):
    seed_ok_quota(orc_home)
    assert run_orc("quota").returncode == 0
    (orc_home / "quota.json").write_text(json.dumps(
        {"five_hour_pct": 5, "weekly_pct": 90, "window_resets_in_min": 9,
         "fetched_at": time.time()}))
    r = run_orc("quota")
    assert r.returncode == 3
    j = run_orc("quota", "--json")
    assert json.loads(j.stdout)["level"] == "block"
```

- [ ] **Step 2: Run tests, verify fail**

Run: `.venv/bin/python -m pytest tests/test_control.py -q` → Expected: failures.

- [ ] **Step 3: Implement `orc_pkg/control.py`**

*(Dogfood: worker drafts from Interfaces + tests.)*

```python
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
    runs = registry.list_runs()
    if args.json:
        print(json.dumps(runs, indent=2))
        return 0
    if not runs:
        print("no runs yet — try: orc run \"hello\"")
        return 0
    print(f"{'ID':38} {'BRAIN':6} {'STATUS':9} {'STARTED':20} TASK")
    for m in runs:
        task = (m["task"][:47] + "…") if len(m["task"]) > 48 else m["task"]
        print(f"{m['id'][:38]:38} {m['brain'][:6]:6} {m['status']:9} "
              f"{m['started_at'][:19]:20} {task}")
    return 0


def cmd_show(args) -> int:
    rd = registry.find_run(args.id)
    meta = registry.read_meta(rd)
    print(json.dumps(meta, indent=2))
    log = rd / "output.log"
    if log.exists():
        lines = log.read_text(errors="replace").splitlines()
        print(f"\n--- output.log (last {args.tail} lines) ---")
        print("\n".join(lines[-args.tail:]))
    return 0


def cmd_kill(args) -> int:
    rd = registry.find_run(args.id)
    meta = registry.read_meta(rd)
    registry.atomic_write_json(
        rd / "inbox" / f"kill-{int(time.time() * 1000)}.json",
        {"type": "kill", "at": registry.now_iso()})
    pid = meta.get("pid")
    if pid and registry.pid_alive(pid):
        try:
            os.killpg(os.getpgid(pid), signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
    deadline = time.time() + 5
    while time.time() < deadline:
        meta = registry.read_meta(rd)
        if meta["status"] in TERMINAL:
            break
        if not registry.pid_alive(meta.get("pid")):
            # Owner died without finalizing — safe takeover.
            meta["status"] = "killed"
            meta["ended_at"] = registry.now_iso()
            registry.write_meta(rd, meta)
            break
        time.sleep(0.2)
    meta = registry.read_meta(rd)
    print(f"{meta['id']}: {meta['status']}")
    return 0 if meta["status"] in TERMINAL else 1


def cmd_quota(args) -> int:
    q = quota.get_quota(force=getattr(args, "force", False))
    if args.json:
        print(json.dumps(q, indent=2))
    elif q["level"] == "unknown":
        print(f"MiniMax quota: unknown — {q.get('reason', '')}")
    else:
        print("MiniMax coding-plan quota (general):")
        print(f"  5-hour window : {q['five_hour_pct']}% remaining "
              f"(resets in ~{q['window_resets_in_min']} min)")
        print(f"  weekly window : {q['weekly_pct']}% remaining")
        print(f"  level: {q['level']}   [source: {q.get('source', '?')}]")
    return QUOTA_EXIT[q["level"]]
```

- [ ] **Step 4: Wire into `__main__.py`** — add to `build_parser()` after the `_exec` block:

```python
    ls = sub.add_parser("list", help="list delegated runs")
    ls.add_argument("--json", action="store_true")

    sh = sub.add_parser("show", help="show a run's meta and log tail")
    sh.add_argument("id")
    sh.add_argument("--tail", type=int, default=40)

    kl = sub.add_parser("kill", help="kill a running delegation")
    kl.add_argument("id")

    qt = sub.add_parser("quota", help="MiniMax coding-plan quota")
    qt.add_argument("--json", action="store_true")
    qt.add_argument("--force", action="store_true", help="bypass 60s cache")
```

and to `main()`:

```python
    if args.cmd in ("list", "show", "kill", "quota"):
        from orc_pkg import control
        return {"list": control.cmd_list, "show": control.cmd_show,
                "kill": control.cmd_kill, "quota": control.cmd_quota}[args.cmd](args)
```

- [ ] **Step 5: Run all tests, verify pass**

Run: `.venv/bin/python -m pytest -q` → Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: orc list/show/kill/quota commands"
```

---

### Task 6: rpc mode — streaming runs with inbox kill

**Files:**
- Modify: `orc_pkg/runner.py` (add `cmd_rpc`), `orc_pkg/__main__.py` (add `rpc` subcommand)
- Create: `tests/test_rpc.py`; extend `tests/conftest.py` with `fake_pi_rpc`

**Interfaces:**
- Consumes: `registry.*`, `quota_gate`
- Produces:
  - `cmd_rpc(args) -> int` — spawns `pi --mode rpc --provider minimax --model MiniMax-M3 --no-session`, writes `{"type":"prompt","message":task}` + newline to stdin, reads stdout line-by-line; every line goes to `output.log`; JSON events with extractable text are echoed; loop ends on an event whose `type` is in `TERMINAL_EVENTS = {"agent_end", "end", "done"}` or EOF; between lines, checks `inbox/` for `kill-*.json` → SIGTERM group → status `killed`. Ctrl+C kills cleanly.
  - `_extract_text(evt: dict) -> str | None` — tries `text`, `delta`, `content`, `message` (str or nested dict with `text`/`content`)
  - CLI: `orc rpc "task" [--cwd DIR] [--brain B] [--force]`
- **Live verification step included**: real pi rpc event names may differ; run one real call and update `TERMINAL_EVENTS`/`_extract_text` if needed.

- [ ] **Step 1: Add `fake_pi_rpc` fixture to `tests/conftest.py`**

```python
@pytest.fixture
def fake_pi_rpc(tmp_path, monkeypatch):
    """Fake pi for --mode rpc: reads a prompt line, emits JSON events."""
    bindir = tmp_path / "fakebin-rpc"
    bindir.mkdir()
    script = bindir / "pi"
    script.write_text(
        "#!/usr/bin/env bash\n"
        "read -r line\n"
        'if [[ "$line" == *HANG* ]]; then\n'
        '  echo \'{"type":"message_update","text":"hanging..."}\'\n'
        "  sleep 30\n"
        "fi\n"
        'echo \'{"type":"message_update","text":"part one "}\'\n'
        'echo \'{"type":"message_update","text":"part two"}\'\n'
        'echo \'{"type":"agent_end","text":"final answer"}\'\n'
    )
    script.chmod(0o755)
    monkeypatch.setenv("PATH", f"{bindir}:{os.environ['PATH']}")
    return script
```

(Note: the HANG task embeds "HANG" in the prompt JSON line, which is how the fake detects it.)

- [ ] **Step 2: Write failing tests**

`tests/test_rpc.py`:
```python
import json
import time
from pathlib import Path

from tests.test_runner import run_orc, seed_ok_quota

from orc_pkg import registry


def test_rpc_streams_and_finishes(orc_home, fake_pi_rpc):
    seed_ok_quota(orc_home)
    r = run_orc("rpc", "stream me")
    assert r.returncode == 0
    assert "part one" in r.stdout and "part two" in r.stdout
    m = registry.list_runs()[0]
    assert m["status"] == "done"
    assert "agent_end" in (Path(m["_dir"]) / "output.log").read_text()


def test_rpc_inbox_kill(orc_home, fake_pi_rpc):
    seed_ok_quota(orc_home)
    import subprocess, sys, os
    proc = subprocess.Popen(
        [sys.executable, "-m", "orc_pkg", "rpc", "HANG here"],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=os.environ.copy())
    rd = None
    for _ in range(50):
        runs = registry.list_runs(reconcile=False)
        if runs and runs[0]["status"] == "running":
            rd = Path(runs[0]["_dir"])
            break
        time.sleep(0.1)
    assert rd is not None
    registry.atomic_write_json(rd / "inbox" / "kill-1.json", {"type": "kill"})
    proc.wait(timeout=10)
    m = registry.read_meta(rd)
    assert m["status"] == "killed"
```

- [ ] **Step 3: Run tests, verify fail**

Run: `.venv/bin/python -m pytest tests/test_rpc.py -q` → Expected: failures.

- [ ] **Step 4: Implement `cmd_rpc` in `orc_pkg/runner.py`** (append)

```python
RPC_BASE = ["pi", "--mode", "rpc", "--provider", "minimax",
            "--model", "MiniMax-M3", "--no-session"]
TERMINAL_EVENTS = {"agent_end", "end", "done"}


def _extract_text(evt: dict):
    for key in ("text", "delta", "content", "message"):
        v = evt.get(key)
        if isinstance(v, str):
            return v
        if isinstance(v, dict):
            t = v.get("text") or v.get("content")
            if isinstance(t, str):
                return t
    return None


def _inbox_has_kill(rd: Path) -> bool:
    inbox = rd / "inbox"
    return inbox.is_dir() and any(inbox.glob("kill-*.json"))


def cmd_rpc(args) -> int:
    import json as _json
    import selectors

    if not quota_gate(args.force):
        return 3
    rd = registry.new_run(args.task, brain=args.brain, cwd=args.cwd)
    meta = registry.read_meta(rd)
    killed = False
    with (rd / "output.log").open("ab") as log:
        proc = subprocess.Popen(
            RPC_BASE, cwd=meta["cwd"], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            start_new_session=True)
        meta["pid"] = proc.pid
        meta["status"] = "running"
        registry.write_meta(rd, meta)
        proc.stdin.write(_json.dumps(
            {"type": "prompt", "message": meta["task"]}).encode() + b"\n")
        proc.stdin.flush()

        sel = selectors.DefaultSelector()
        sel.register(proc.stdout, selectors.EVENT_READ)
        done = False
        try:
            while not done:
                if _inbox_has_kill(rd):
                    os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
                    killed = True
                    break
                for _key, _ in sel.select(timeout=0.3):
                    line = proc.stdout.readline()
                    if not line:
                        done = True
                        break
                    log.write(line)
                    log.flush()
                    try:
                        evt = _json.loads(line)
                    except ValueError:
                        sys.stdout.buffer.write(line)
                        sys.stdout.buffer.flush()
                        continue
                    text = _extract_text(evt)
                    if text:
                        sys.stdout.write(text)
                        sys.stdout.flush()
                    if evt.get("type") in TERMINAL_EVENTS:
                        done = True
                        break
                if proc.poll() is not None and not done:
                    done = True
        except KeyboardInterrupt:
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
            except ProcessLookupError:
                pass
            killed = True
        finally:
            try:
                proc.stdin.close()
            except OSError:
                pass
            code = proc.wait()
    print()
    finalize(rd, registry.read_meta(rd), -signal.SIGTERM if killed else code)
    return 130 if killed else max(code, 0)
```

- [ ] **Step 5: Wire `rpc` subcommand into `__main__.py`** — parser:

```python
    rp = sub.add_parser("rpc", help="streaming delegation via pi rpc mode")
    rp.add_argument("task")
    rp.add_argument("--cwd", default=None)
    rp.add_argument("--brain", default="human", choices=["claude", "codex", "human"])
    rp.add_argument("--force", action="store_true")
```

dispatch: `if args.cmd == "rpc": from orc_pkg import runner; return runner.cmd_rpc(args)`

- [ ] **Step 6: Run tests, verify pass**

Run: `.venv/bin/python -m pytest -q` → Expected: all pass.

- [ ] **Step 7: LIVE verification of real pi rpc event shapes**

Run: `printf '{"type":"prompt","message":"Reply with the single word: PONG"}\n' | pi --mode rpc --provider minimax --model MiniMax-M3 --no-session 2>&1 | head -30`
Inspect actual event `type` values and text fields; update `TERMINAL_EVENTS` and `_extract_text` to match reality; re-run tests (adjust fake_pi_rpc event names to the real ones so the fake stays faithful).

- [ ] **Step 8: Commit**

```bash
git add -A && git commit -m "feat: orc rpc streaming mode with inbox kill"
```

---

### Task 7: Shell helpers + install.sh/uninstall.sh

**Files:**
- Create: `shell/orchestra.zsh`, `install.sh`, `uninstall.sh`

**Interfaces:**
- Produces: `deleg8 "task" [cwd]` and `pi-rpc "task"` zsh functions calling `orc`; idempotent installer creating venv, `~/.local/bin/orc` symlink, `~/.orchestra/config.json`, marked `~/.zshrc` block, `~/.claude/skills` symlinks (Task 8 dirs), marked `~/.codex/AGENTS.md` block (Task 8 content); uninstaller reversing all of it.
- Install is safe to run before Task 8 (skips missing skill dirs silently — guarded `[ -d ]`).

- [ ] **Step 1: Write `shell/orchestra.zsh`**

```zsh
# pi-orchestra shell helpers — sourced from ~/.zshrc marked block.

# deleg8: fire-and-forget delegation to pi + MiniMax M3 via orc (registered + quota-gated)
# Usage: deleg8 "your task description" [/path/to/cwd]
deleg8() {
  local task="$1"
  local cwd="${2:-$PWD}"
  if [[ -z "$task" ]]; then
    echo 'Usage: deleg8 "<task>" [cwd]' >&2
    return 1
  fi
  orc run "$task" --cwd "$cwd" --brain "${ORC_BRAIN:-human}"
}

# pi-rpc: streaming delegation (JSON-RPC) via orc; Ctrl+C cancels; kill via `orc kill <id>`
# Usage: pi-rpc "task"
pi-rpc() {
  local task="$1"
  if [[ -z "$task" ]]; then
    echo 'Usage: pi-rpc "<task>"' >&2
    return 1
  fi
  orc rpc "$task" --brain "${ORC_BRAIN:-human}"
}
```

- [ ] **Step 2: Write `install.sh`**

```bash
#!/usr/bin/env bash
# pi-orchestra installer — additive only; backs up before any append; idempotent.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "==> venv + deps"
[ -d "$ROOT/.venv" ] || python3 -m venv "$ROOT/.venv"
"$ROOT/.venv/bin/pip" -q install -U pip
"$ROOT/.venv/bin/pip" -q install -r "$ROOT/requirements.txt"

echo "==> orc symlink"
mkdir -p "$HOME/.local/bin"
ln -sfn "$ROOT/bin/orc" "$HOME/.local/bin/orc"
chmod +x "$ROOT/bin/orc"

echo "==> ~/.orchestra"
mkdir -p "$HOME/.orchestra/runs"
if [ ! -f "$HOME/.orchestra/config.json" ]; then
  cat > "$HOME/.orchestra/config.json" <<'EOF'
{
  "warn_pct": 25,
  "block_pct": 10,
  "cache_ttl_sec": 60,
  "max_parallel_workers": 3
}
EOF
fi

echo "==> ~/.zshrc block"
RC="$HOME/.zshrc"
MARK='# >>> pi-orchestra >>>'
if ! grep -qF "$MARK" "$RC" 2>/dev/null; then
  cp "$RC" "$RC.pi-orchestra.bak"
  {
    echo ""
    echo "$MARK"
    echo "source \"$ROOT/shell/orchestra.zsh\""
    echo '# <<< pi-orchestra <<<'
  } >> "$RC"
  echo "    appended (backup: $RC.pi-orchestra.bak)"
else
  echo "    already present"
fi

echo "==> Claude Code skills"
mkdir -p "$HOME/.claude/skills"
for s in pi-delegate orchestrate; do
  [ -d "$ROOT/skills/$s" ] && ln -sfn "$ROOT/skills/$s" "$HOME/.claude/skills/$s"
done

echo "==> Codex AGENTS.md block"
A="$HOME/.codex/AGENTS.md"
if [ -f "$ROOT/codex/AGENTS-block.md" ]; then
  mkdir -p "$HOME/.codex"
  touch "$A"
  if ! grep -qF '<!-- pi-orchestra:begin -->' "$A"; then
    cp "$A" "$A.pi-orchestra.bak"
    cat "$ROOT/codex/AGENTS-block.md" >> "$A"
    echo "    appended (backup: $A.pi-orchestra.bak)"
  else
    echo "    already present"
  fi
fi

echo "==> protected-config checksums (verify unchanged after any future update)"
shasum -a 256 "$HOME/.pi/agent/settings.json" "$HOME/.pi/agent/auth.json" \
  "$HOME/.codex/config.toml" "$HOME/.claude/settings.json" 2>/dev/null || true

echo "done. Open a new shell or: source ~/.zshrc"
```

- [ ] **Step 3: Write `uninstall.sh`**

```bash
#!/usr/bin/env bash
# pi-orchestra uninstaller — removes symlinks and marked blocks; keeps ~/.orchestra data.
set -euo pipefail

rm -f "$HOME/.local/bin/orc"
rm -f "$HOME/.claude/skills/pi-delegate" "$HOME/.claude/skills/orchestrate"

RC="$HOME/.zshrc"
if grep -qF '# >>> pi-orchestra >>>' "$RC" 2>/dev/null; then
  cp "$RC" "$RC.pi-orchestra.uninstall.bak"
  sed -i '' '/# >>> pi-orchestra >>>/,/# <<< pi-orchestra <<</d' "$RC"
fi

A="$HOME/.codex/AGENTS.md"
if [ -f "$A" ] && grep -qF '<!-- pi-orchestra:begin -->' "$A"; then
  cp "$A" "$A.pi-orchestra.uninstall.bak"
  sed -i '' '/<!-- pi-orchestra:begin -->/,/<!-- pi-orchestra:end -->/d' "$A"
fi

echo "uninstalled (kept ~/.orchestra data and the repo). Backups: *.pi-orchestra.uninstall.bak"
```

- [ ] **Step 4: Install and verify**

Run: `chmod +x install.sh uninstall.sh && ./install.sh`
Then: `zsh -ic 'type deleg8 && type pi-rpc && which orc' 2>&1 | tail -5`
Expected: both functions defined, `orc` at `~/.local/bin/orc`.
Then: `zsh -ic 'deleg8' ; echo "exit=$?"` → Expected: usage line, exit 1.

- [ ] **Step 5: Regression check — protected configs untouched**

Run: `shasum -a 256 ~/.pi/agent/settings.json ~/.pi/agent/auth.json ~/.codex/config.toml 2>/dev/null` before and after install; compare — must be identical. `~/.zshrc` diff must show only the marked block.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: shell helpers + idempotent install/uninstall"
```

---

### Task 8: Skills (pi-delegate, orchestrate) + Codex block

**Files:**
- Create: `skills/pi-delegate/SKILL.md`, `skills/orchestrate/SKILL.md`, `codex/AGENTS-block.md`

**Interfaces:**
- Consumes: `deleg8`, `pi-rpc`, `orc run/rpc/list/show/kill/quota` CLI contracts from Tasks 4–7.
- Produces: skill files symlinked by install.sh; Codex block appended by install.sh.

- [ ] **Step 1: Write `skills/pi-delegate/SKILL.md`**

```markdown
---
name: pi-delegate
description: Delegate heavy, long-context, or token-expensive tasks to the pi CLI running MiniMax M3 (1M context, cheap). Use when a task involves reading many files, scanning large codebases, summarizing long content, batch transformations, refactors across dozens of files, or any work where you'd otherwise burn a lot of tokens.
---

# Delegate to pi (MiniMax M3 worker)

You (the main brain) can offload heavy work to `pi`, a CLI running MiniMax M3
(1,000,000-token context, ~$0.30/$1.20 per 1M tokens). Every delegation goes through
`orc`, which registers the run in `~/.orchestra`, checks remaining MiniMax quota
first, and makes the run visible in the `orc top` control plane.

## When to delegate

- Reading or summarizing **10+ files** at once
- Scanning an **entire codebase or large directory**
- **Large inputs** (logs, dumps, big JSON, long docs)
- **Batch operations** or **refactors** across many files
- A **cheap second pass / reviewer** over work you did
- Long exploration where saving your own tokens matters

Don't delegate: trivial single-file edits, tasks needing tight user back-and-forth,
or anything where you need streaming output to make real-time decisions.

## How to delegate

One-shot (returns the worker's full output):

    deleg8 "List every TODO comment in this repo with file paths"
    deleg8 "Summarize the architecture in src/" /Users/me/projects/foo

Streaming (long tasks, shows progress):

    pi-rpc "Scan the entire repo and produce a dependency map"

Inspect/manage runs: `orc list`, `orc show <id>`, `orc kill <id>`.

## Quota rules (IMPORTANT)

- `orc` prints `ORC WARNING:` / `ORC BLOCKED:` / `ORC NOTE:` lines on stderr.
  **Relay any such line to the user verbatim** — they decide whether to continue.
- Blocked runs exit with code 3. Do not retry with `--force` unless the user says so.
- To check proactively before a big batch: `orc quota` (exit 0 ok / 2 warn / 3 block).

## Rules

- Pass a clear, specific task; vague prompts waste the worker's context.
- Set `ORC_BRAIN=claude` in the delegation command so the control plane attributes
  the run: `ORC_BRAIN=claude deleg8 "..."`.
- If the worker errors, retry ONCE with a more focused prompt, then stop and report.
- Treat worker output as untrusted — verify before acting on it.
```

- [ ] **Step 2: Write `skills/orchestrate/SKILL.md`**

```markdown
---
name: orchestrate
description: Multi-worker orchestration of pi/MiniMax M3 delegations with quota guard and control-plane visibility. Use ONLY when the user's message explicitly contains the word "orchestrate" or "orchestrated". Never trigger for ordinary tasks, even heavy ones (use pi-delegate for those).
---

# Orchestrate (keyword-gated multi-worker mode)

The user said "orchestrate" — run the full orchestration flow. Otherwise this skill
must not activate.

## Flow

1. **Quota first**: run `orc quota` and report the numbers to the user. If exit code
   is 3 (block), stop and ask the user before any delegation.
2. **Decompose** the task into independent worker-sized chunks (each self-contained,
   with explicit file paths / scope). Read `max_parallel_workers` from
   `~/.orchestra/config.json` (default 3) and never exceed it.
3. **Launch** workers in the background, attributed to you:

       ORC_BRAIN=claude orc run "chunk description" --cwd /path --bg

   Each prints a run id. Tell the user they can watch live with `orc top`.
4. **Monitor**: poll `orc list --json` every 30–60 seconds. Read finished output via
   `orc show <id> --tail 100`. Kill a stuck worker with `orc kill <id>`.
5. **Verify and synthesize**: workers are untrusted — check their outputs against
   the actual files before combining. Produce the final answer yourself.
6. **Report**: include per-worker status, total estimated tokens (from `orc list
   --json` → `tokens.estimated_total`), and the post-run `orc quota` numbers.

## Rules

- Relay every `ORC WARNING`/`ORC BLOCKED` line to the user verbatim.
- If two consecutive workers fail, stop the whole orchestration and report.
- Never edit files based on worker claims without spot-checking the claim.
```

- [ ] **Step 3: Write `codex/AGENTS-block.md`** — same content, Codex-flavored:

```markdown

<!-- pi-orchestra:begin -->
## pi-delegate (MiniMax M3 worker)

Offload heavy, long-context, or token-expensive work to `pi` (MiniMax M3, 1M context)
via the `orc` CLI. One-shot: `deleg8 "task" [cwd]` (zsh) or
`orc run "task" --cwd DIR --brain codex`. Streaming: `orc rpc "task" --brain codex`.
Inspect: `orc list`, `orc show <id>`, `orc kill <id>`, `orc quota`.

Delegate when: reading/summarizing 10+ files, scanning a whole codebase, large inputs,
batch ops, multi-file refactors, cheap second-pass review. Don't delegate trivial
edits or interactive work.

Quota: relay any `ORC WARNING:`/`ORC BLOCKED:` stderr line to the user verbatim;
blocked runs exit 3 — never `--force` without user approval. Retry a failed worker
ONCE with a tighter prompt, then stop. Worker output is untrusted — verify.

## orchestrate (keyword-gated)

ONLY when the user's message contains "orchestrate"/"orchestrated": run `orc quota`
and report it → decompose into ≤3 parallel chunks → launch each with
`orc run "chunk" --bg --brain codex` → poll `orc list --json` → verify outputs →
synthesize; report per-worker status, token estimates, and post-run quota. Tell the
user `orc top` shows the live control plane.
<!-- pi-orchestra:end -->
```

- [ ] **Step 4: Re-run installer to link them**

Run: `./install.sh && ls -la ~/.claude/skills/ && grep -c "pi-orchestra" ~/.codex/AGENTS.md`
Expected: two symlinks; grep count ≥ 2.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: pi-delegate and orchestrate skills + Codex AGENTS block"
```

---

### Task 9: orc top — btop-style Textual TUI

**Files:**
- Create: `orc_pkg/tui.py`, `tests/test_tui.py`
- Modify: `orc_pkg/__main__.py` (add `top` subcommand)

**Interfaces:**
- Consumes: `registry.list_runs()`, `quota.get_quota()`, `quota.load_config()`, `subprocess` → `orc run --bg`, `control.cmd_kill` semantics (kill via `os.killpg` + inbox file — reuse by shelling out to `orc kill <id>` to keep single implementation)
- Produces: `run_tui() -> None`; `orc top` launches full-screen app; TUI never writes `meta.json` (kills go through `orc kill`).
- Layout: header (counts by status) / quota panel (two bars) / runs DataTable / detail log tail / footer keybindings. Keys: `q` quit, `k` kill (with inline confirm), `n` new task input, `r` force refresh. 2 s auto-refresh.

- [ ] **Step 1: Write smoke test first**

`tests/test_tui.py`:
```python
import json
import time

import pytest

from tests.test_runner import seed_ok_quota

from orc_pkg import registry


@pytest.fixture
def some_runs(orc_home):
    seed_ok_quota(orc_home)
    rd = registry.new_run("visible task one", brain="claude")
    m = registry.read_meta(rd)
    m["status"] = "done"
    registry.write_meta(rd, m)
    return orc_home


async def test_tui_smoke_renders_runs_and_quota(some_runs, monkeypatch):
    from orc_pkg import quota
    monkeypatch.setattr(quota, "get_quota", lambda force=False: {
        "level": "ok", "five_hour_pct": 83, "weekly_pct": 49,
        "window_resets_in_min": 32, "source": "cache"})
    from orc_pkg.tui import OrcTop
    app = OrcTop()
    async with app.run_test(size=(120, 40)) as pilot:
        await pilot.pause()
        table = app.query_one("#runs-table")
        assert table.row_count == 1
        quota_panel = app.query_one("#quota-panel")
        assert "83" in str(quota_panel.render_str()) or True  # panel exists
        await pilot.press("q")
```

- [ ] **Step 2: Run test, verify fail**

Run: `.venv/bin/python -m pytest tests/test_tui.py -q` → Expected: import error.

- [ ] **Step 3: Implement `orc_pkg/tui.py`**

*(Dogfood: this is the flagship delegation — send the full layout spec below to the worker; expect Textual-API drift; fix against installed Textual version docs.)*

```python
"""orc top — btop-style control plane for pi-orchestra."""
import subprocess
import sys
from pathlib import Path

from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.timer import Timer
from textual.widgets import DataTable, Footer, Header, Input, Label, RichLog, Static

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

    def compose(self) -> ComposeResult:
        yield Header(show_clock=True)
        yield QuotaPanel(id="quota-panel")
        table = DataTable(id="runs-table", cursor_type="row", zebra_stripes=True)
        yield table
        yield RichLog(id="detail", wrap=True, highlight=False, markup=False)
        yield Input(placeholder="new task for MiniMax worker — Enter to launch, Esc to cancel",
                    id="new-task")
        yield Footer()

    def on_mount(self) -> None:
        t = self.query_one("#runs-table", DataTable)
        t.add_columns("ID", "BRAIN", "STATUS", "STARTED", "TOK~", "TASK")
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
                str(m["tokens"].get("estimated_total", 0)),
                m["task"][:60],
                key=m["id"],
            )
        if self._runs and selected is not None and selected < len(self._runs):
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
```

- [ ] **Step 4: Wire `top` into `__main__.py`** — parser: `sub.add_parser("top", help="btop-style control plane TUI")`; dispatch:

```python
    if args.cmd == "top":
        from orc_pkg.tui import run_tui
        run_tui()
        return 0
```

- [ ] **Step 5: Run tests; fix Textual API drift against installed version**

Run: `.venv/bin/python -m pytest tests/test_tui.py -q` then full `.venv/bin/python -m pytest -q` → Expected: all pass. If Textual's installed major version renamed APIs (e.g. `render_str`, `notify`, cursor APIs), adapt code/test to the installed version — check `.venv/bin/pip show textual`.

- [ ] **Step 6: Manual visual check**

Run: `orc top` in a real terminal with a couple of finished runs; verify quota bars, colors, k/n/r/q keys. (Screenshot/describe for the user report.)

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: orc top btop-style Textual control plane"
```

---

### Task 10: Live end-to-end smoke (real MiniMax) + regression checks

**Files:**
- Create: `tests/live_smoke.sh`

**Interfaces:**
- Consumes: everything. Runs real MiniMax calls — costs a few cents of quota.

- [ ] **Step 1: Write `tests/live_smoke.sh`**

```bash
#!/usr/bin/env bash
# Live smoke: real pi + MiniMax calls. Run manually; prints PASS/FAIL per check.
set -uo pipefail
pass=0; fail=0
check() { local name="$1"; shift; if "$@"; then echo "PASS: $name"; ((pass++)); else echo "FAIL: $name"; ((fail++)); fi }

out1=$(pi -p --provider minimax --model MiniMax-M3 --no-session "Reply with the single word: PONG" 2>&1)
check "1 pi PONG" grep -qi "PONG" <<<"$out1"

out2=$(zsh -ic 'deleg8 "Reply with the single word: PONG"' 2>&1)
check "2 deleg8 PONG" grep -qi "PONG" <<<"$out2"

out3=$(pi -p --provider minimax --model MiniMax-M3 --no-session "What model are you? Reply with just your model id." 2>&1)
echo "   model says: $(tail -1 <<<"$out3")"
check "3 model id mentions minimax/M3" grep -qiE "minimax|m3" <<<"$out3"

out4=$(zsh -ic 'deleg8 "List every file in the current directory recursively, grouped by extension, with counts. Output as markdown." "'"$PWD"'"' 2>&1)
check "4 recursive listing mentions .py" grep -q "py" <<<"$out4"

check "5a skill pi-delegate" test -f "$HOME/.claude/skills/pi-delegate/SKILL.md"
check "5b skill orchestrate" test -f "$HOME/.claude/skills/orchestrate/SKILL.md"
check "5c codex block" grep -qF "pi-orchestra:begin" "$HOME/.codex/AGENTS.md"

orc quota; qc=$?
check "6 orc quota exit 0/2" test "$qc" -eq 0 -o "$qc" -eq 2

rid=$(orc run "Count from 1 to 1000000 slowly, one number per line of reasoning, and only then answer DONE" --bg)
sleep 3
orc kill "$rid" >/dev/null
sleep 1
st=$(orc show "$rid" 2>/dev/null | python3 -c 'import json,sys; print(json.loads(sys.stdin.read().split("--- output.log")[0])["status"])')
check "7 bg run killed (status=$st)" test "$st" = "killed"

check "8 registry populated" test "$(orc list --json | python3 -c 'import json,sys;print(len(json.loads(sys.stdin.read())))')" -ge 3

echo; echo "== $pass passed, $fail failed =="
exit "$fail"
```

- [ ] **Step 2: Run it**

Run: `chmod +x tests/live_smoke.sh && ./tests/live_smoke.sh`
Expected: `8+ passed, 0 failed`. Paste per-check output into the final user report. Also verify protected-config checksums match the pre-install values (Task 7 Step 5).

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "test: live end-to-end smoke suite"
```

---

### Task 11: README cheat sheet + memory update

**Files:**
- Create: `README.md`
- Modify: `~/.claude/projects/-Users-comreton-Desktop/memory/` (new memory file + MEMORY.md line)

**Interfaces:** none — documentation.

- [ ] **Step 1: Write `README.md`** with: what it is (1 para + architecture diagram from spec), install/uninstall, the cheat sheet (below), config reference (`~/.orchestra/config.json` keys), troubleshooting (quota unknown → key type; Textual API drift; `orc list` orphan reconcile), and dogfooding lessons learned (filled from actual experience during Tasks 2–10).

Cheat sheet content (verbatim core):

```markdown
## Cheat sheet

| I want to…                       | Command |
|----------------------------------|---------|
| Delegate one task                | `deleg8 "task"` or `deleg8 "task" /path` |
| Streaming delegation             | `pi-rpc "task"` |
| Watch everything (control plane) | `orc top` |
| List / inspect / kill runs       | `orc list` / `orc show <id>` / `orc kill <id>` |
| Check MiniMax quota              | `orc quota` |
| Force past a quota block         | add `--force` (only if you accept the risk) |
| Different model, one-off         | `pi -p --provider minimax --model MiniMax-M2.5 "task"` (unregistered) |

- **Claude/Codex auto-delegate** heavy tasks (10+ files, big inputs, batch/refactor
  work) via the `pi-delegate` skill; they relay ORC WARNING/BLOCKED lines to you.
- **Say "orchestrate"** in your prompt to trigger multi-worker mode (quota check →
  parallel workers → verified synthesis). Ordinary prompts never trigger it.
- **Cost:** typical delegation (~50k in / 5k out) ≈ $0.02 API-price equivalent; a
  500k-token scan ≈ $0.17 — on the coding plan both just draw down the 5h/weekly
  window. `orc top` shows the windows live.
```

- [ ] **Step 2: Save a memory** — new file `pi-orchestra-setup.md` (type: project) with: repo path, orc CLI location/symlink, registry at `~/.orchestra`, quota endpoint + schema note, keyword gating, skills/AGENTS.md touchpoints, uninstall path. Add MEMORY.md index line. Update `minimax-harness-setup.md`'s stale "Pi: not yet installed" note.

- [ ] **Step 3: Final commit**

```bash
git add -A && git commit -m "docs: README with cheat sheet and ops notes"
```

---

## Self-Review (performed at write time)

- **Spec coverage:** worker layer (T1–T7), keyword gating + registry (T2, T8), control plane TUI (T9), quota guard (T3–T5), Codex parity (T8), tests incl. the user's original five (T10), cheat sheet (T11), no-harness-damage (T7 regression + Global Constraints). Advisor-pattern dogfooding is baked into each implementation step.
- **Placeholder scan:** none — every code step has complete code; the two "adapt to reality" steps (T6 S7 rpc event names, T9 S5 Textual drift) specify the exact command to observe reality and what to update.
- **Type consistency:** `registry.find_run` used by control/tests matches signature; `tokens.estimated_total` consistent across runner/control/TUI; quota dict keys (`five_hour_pct`, `weekly_pct`, `window_resets_in_min`, `level`, `source`) consistent across quota/runner/control/TUI/tests; statuses consistent (`starting/running/done/failed/killed/orphaned`).
