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
    orc_home.mkdir(parents=True, exist_ok=True)
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
    m = None
    for _ in range(50):
        m = registry.read_meta(registry.find_run(run_id))
        if m["status"] == "done":
            break
        time.sleep(0.2)
    assert m["status"] == "done"


def test_run_idle_timeout_kills_stalled_worker(orc_home, fake_pi):
    seed_ok_quota(orc_home)
    start = time.time()
    r = run_orc("run", "please SLEEP forever", "--idle-timeout", "2")
    elapsed = time.time() - start
    assert elapsed < 15
    m = registry.list_runs()[0]
    assert m["status"] == "failed"
    assert m["exit_code"] == 124
    log = (Path(m["_dir"]) / "output.log").read_text()
    assert "idle timeout" in log
    assert not registry.pid_alive(m["pid"])
