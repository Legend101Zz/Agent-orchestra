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


def test_new_run_with_session(orc_home):
    rd = registry.new_run("t", session="orch-123")
    assert registry.read_meta(rd)["session"] == "orch-123"


def test_new_run_without_session_omits_field(orc_home):
    rd = registry.new_run("t")
    assert "session" not in registry.read_meta(rd)


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
