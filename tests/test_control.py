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
    m = None
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
