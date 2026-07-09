import os
import subprocess
import sys
import time
from pathlib import Path

from tests.test_runner import run_orc, seed_ok_quota

from orc_pkg import registry


def test_rpc_streams_and_finishes(orc_home, fake_pi_rpc):
    seed_ok_quota(orc_home)
    r = run_orc("rpc", "stream me")
    assert r.returncode == 0
    assert "part one " in r.stdout and "part two" in r.stdout
    m = registry.list_runs()[0]
    assert m["status"] == "done"
    assert "agent_end" in (Path(m["_dir"]) / "output.log").read_text()
    assert m["tokens"]["input"] == 84
    assert m["tokens"]["total"] == 1639
    assert m["tokens"]["cost_usd"] == 0.00014
    assert m["tokens"]["estimated_total"] == 1639


def test_rpc_inbox_kill(orc_home, fake_pi_rpc):
    seed_ok_quota(orc_home)
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
