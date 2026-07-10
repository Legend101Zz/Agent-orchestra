import json
import os
import stat
import subprocess
import time
from pathlib import Path

import pytest

from orc_pkg import registry
from tests.test_runner import seed_ok_quota


ROOT = Path(__file__).resolve().parents[1]
RUST_BIN = ROOT / "rust" / "target" / "debug" / "orc"


@pytest.fixture(scope="session")
def rust_orc():
    subprocess.run(
        ["cargo", "build", "--manifest-path", str(ROOT / "rust/Cargo.toml"), "-q"],
        check=True,
    )
    assert RUST_BIN.is_file()
    return RUST_BIN


def run_rust(rust_orc, *args, env=None, **kwargs):
    return subprocess.run(
        [str(rust_orc), *args],
        capture_output=True,
        text=True,
        env=env if env is not None else os.environ.copy(),
        **kwargs,
    )


def wait_terminal(run_dir, timeout=10):
    deadline = time.time() + timeout
    while time.time() < deadline:
        meta = registry.read_meta(run_dir)
        if meta["status"] in ("done", "failed", "killed", "orphaned"):
            return meta
        time.sleep(0.05)
    raise AssertionError(f"run did not finish: {run_dir}")


def test_rust_run_is_readable_by_python(orc_home, fake_pi_json, rust_orc):
    seed_ok_quota(orc_home)
    result = run_rust(rust_orc, "run", "rust writes", "--brain", "codex")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "json part one json part two\n"
    runs = registry.list_runs()
    assert len(runs) == 1
    meta = runs[0]
    assert meta["status"] == "done"
    assert meta["brain"] == "codex"
    assert meta["tokens"]["total"] == 2198
    assert meta["tokens"]["cost_usd"] == 0.000201
    log = (Path(meta["_dir"]) / "output.log").read_text()
    update = next(json.loads(line) for line in log.splitlines() if "message_update" in line)
    assert "message" not in update  # Rust removes the quadratic cumulative snapshot.


def test_rust_list_reads_python_run(orc_home, rust_orc):
    run_dir = registry.new_run("python writes", brain="claude")
    meta = registry.read_meta(run_dir)
    meta.update(status="done", ended_at=registry.now_iso(), exit_code=0)
    registry.write_meta(run_dir, meta)
    result = run_rust(rust_orc, "list", "--json")
    assert result.returncode == 0, result.stderr
    data = json.loads(result.stdout)
    assert data[0]["id"] == meta["id"]
    assert data[0]["status"] == "done"
    assert data[0]["_dir"] == str(run_dir)


def test_rpc_send_delivers_once_and_acks(orc_home, tmp_path, monkeypatch, rust_orc):
    seed_ok_quota(orc_home)
    bindir = tmp_path / "fake-rpc-steer"
    bindir.mkdir()
    script = bindir / "pi"
    script.write_text(
        "#!/usr/bin/env bash\n"
        "read -r initial\n"
        "echo '{\"type\":\"agent_start\"}'\n"
        "read -r followup\n"
        "echo \"$followup\" > \"$ORC_HOME/followup.txt\"\n"
        "echo '{\"type\":\"message_update\",\"assistantMessageEvent\":{\"type\":\"text_delta\",\"delta\":\"steered\"}}'\n"
        "echo '{\"type\":\"agent_end\",\"messages\":[{\"role\":\"assistant\",\"usage\":{\"input\":10,\"output\":2,\"cacheRead\":0,\"totalTokens\":12,\"cost\":{\"total\":0.00001}}}]}'\n"
    )
    script.chmod(script.stat().st_mode | stat.S_IEXEC)
    monkeypatch.setenv("PATH", f"{bindir}:{os.environ['PATH']}")
    started = run_rust(rust_orc, "rpc", "initial", "--bg", "--brain", "codex")
    assert started.returncode == 0, started.stderr
    run_id = started.stdout.strip()
    run_dir = registry.find_run(run_id)
    deadline = time.time() + 5
    while registry.read_meta(run_dir)["status"] != "running" and time.time() < deadline:
        time.sleep(0.05)
    sent = run_rust(rust_orc, "send", run_id, "focus on tests")
    assert sent.returncode == 0, sent.stderr
    meta = wait_terminal(run_dir)
    assert meta["status"] == "done"
    delivered = json.loads((orc_home / "followup.txt").read_text())
    assert delivered == {"type": "prompt", "message": "focus on tests"}
    acks = list((run_dir / "inbox" / "processed").glob("ack-*.json"))
    assert len(acks) == 1


def test_retry_and_handoff_are_additive(orc_home, fake_pi_json, rust_orc):
    seed_ok_quota(orc_home)
    first = run_rust(rust_orc, "run", "original")
    assert first.returncode == 0
    original = registry.list_runs()[0]
    retry = run_rust(rust_orc, "retry", original["id"])
    assert retry.returncode == 0, retry.stderr
    retry_dir = registry.find_run(retry.stdout.strip())
    retry_meta = wait_terminal(retry_dir)
    assert retry_meta["retry_of"] == original["id"]
    handoff = run_rust(rust_orc, "handoff", original["id"], "finish the missing tests")
    assert handoff.returncode == 0, handoff.stderr
    handoff_dir = registry.find_run(handoff.stdout.strip())
    handoff_meta = wait_terminal(handoff_dir)
    assert handoff_meta["handoff_from"] == original["id"]
    assert registry.read_meta(Path(original["_dir"])).get("retry_of") is None
