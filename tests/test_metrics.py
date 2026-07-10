import json
from datetime import datetime, timezone

from orc_pkg import metrics, registry

EXACT = {"id": "a", "brain": "claude", "status": "done", "session": "s1",
         "started_at": "2026-07-10T10:00:00+00:00",
         "tokens": {"input": 100000, "output": 10000, "cache_read": 0,
                    "total": 110000, "cost_usd": 0.042, "estimated_total": 110000}}
EST = {"id": "b", "brain": "codex", "status": "failed",
       "started_at": "2026-07-09T10:00:00+00:00",
       "tokens": {"estimated_total": 8000}}
OLD = {"id": "c", "brain": "human", "status": "done",
       "started_at": "2026-07-08T10:00:00+00:00"}   # pre-v2 meta: no tokens at all


def test_worker_stats_split_exact_vs_estimated():
    s = metrics.worker_stats([EXACT, EST, OLD])
    assert s["runs"] == 3
    assert s["exact"]["runs"] == 1 and s["exact"]["total"] == 110000
    assert s["exact"]["cost_usd"] == 0.042
    assert s["estimated"]["runs"] == 2 and s["estimated"]["total"] == 8000
    assert s["by_brain"]["claude"]["total"] == 110000
    assert s["by_session"]["s1"]["runs"] == 1
    assert s["by_day"]["2026-07-10"]["runs"] == 1


def test_worker_stats_by_status():
    s = metrics.worker_stats([EXACT, EST, OLD])
    assert s["by_status"] == {"done": 2, "failed": 1}


def test_delegated_value_prices_at_brain_rates():
    v = metrics.delegated_value([EXACT])
    # claude equiv: 100k/1M*3 + 10k/1M*15 = 0.30 + 0.15 = 0.45
    assert round(v["brain_equiv_usd"], 4) == 0.45
    assert v["worker_cost_usd"] == 0.042
    assert v["saved_usd"] == round(0.45 - 0.042, 4)
    assert v["multiple"] == round(0.45 / 0.042, 1)
    assert v["exact_share"] == 1.0


def test_delegated_value_empty_is_zero():
    v = metrics.delegated_value([])
    assert v["saved_usd"] == 0 and v["multiple"] == 0


def test_delegated_value_estimated_runs_lower_exact_share():
    v = metrics.delegated_value([EXACT, EST])
    assert 0 < v["exact_share"] < 1
    assert v["brain_equiv_usd"] > 0.45   # EST tokens priced too (as input)


def make_claude_line(ts, model="claude-sonnet-5", inp=100, out=50, mid=None):
    return json.dumps({"timestamp": ts, "type": "assistant",
        "requestId": mid or f"req-{ts}-{inp}",
        "message": {"id": mid or f"m-{ts}-{inp}", "model": model,
            "usage": {"input_tokens": inp, "output_tokens": out,
                      "cache_read_input_tokens": 7, "cache_creation_input_tokens": 3}}})


NOW = datetime(2026, 7, 10, 12, tzinfo=timezone.utc)


def test_brain_usage_parses_claude_jsonl(tmp_path, orc_home):
    proj = tmp_path / "projects" / "-Users-x-proj"
    proj.mkdir(parents=True)
    today = "2026-07-10T09:00:00.000Z"
    old = "2026-06-01T09:00:00.000Z"
    (proj / "s1.jsonl").write_text(
        make_claude_line(today) + "\n" + "GARBAGE not json\n" + make_claude_line(old) + "\n")
    u = metrics.brain_usage(claude_dir=tmp_path / "projects",
                            codex_dir=tmp_path / "nope", now=NOW)
    assert u["claude"]["today"]["input"] == 100
    assert u["claude"]["today"]["output"] == 50
    assert u["claude"]["by_model"]["claude-sonnet-5"] >= 150
    assert u["codex"] is None


def test_brain_usage_dedups_by_message_id(tmp_path, orc_home):
    proj = tmp_path / "projects" / "p"
    proj.mkdir(parents=True)
    line = make_claude_line("2026-07-10T09:00:00.000Z", mid="dup-1")
    (proj / "s.jsonl").write_text(line + "\n" + line + "\n")
    u = metrics.brain_usage(claude_dir=tmp_path / "projects",
                            codex_dir=tmp_path / "nope", now=NOW)
    assert u["claude"]["today"]["input"] == 100   # counted once


def test_brain_usage_cache_hits_on_unchanged_mtime(tmp_path, orc_home):
    proj = tmp_path / "projects" / "p"
    proj.mkdir(parents=True)
    f = proj / "s.jsonl"
    f.write_text(make_claude_line("2026-07-10T09:00:00.000Z") + "\n")
    kw = dict(claude_dir=tmp_path / "projects", codex_dir=tmp_path / "nope", now=NOW)
    first = metrics.brain_usage(**kw)
    cache = json.loads((registry.home() / "brain_usage_cache.json").read_text())
    assert str(f) in cache["files"]
    second = metrics.brain_usage(**kw)     # second call served from cache
    assert second["claude"]["today"] == first["claude"]["today"]


def test_brain_usage_parses_codex_token_counts(tmp_path, orc_home):
    day = tmp_path / "sessions" / "2026" / "07" / "10"
    day.mkdir(parents=True)
    lines = [
        json.dumps({"timestamp": "2026-07-10T08:00:00.000Z", "type": "event_msg",
                    "payload": {"type": "token_count", "info": None}}),
        json.dumps({"timestamp": "2026-07-10T08:01:00.000Z", "type": "event_msg",
                    "payload": {"type": "token_count", "info": {"total_token_usage": {
                        "input_tokens": 1000, "cached_input_tokens": 400,
                        "output_tokens": 100, "total_tokens": 1100}}}}),
        json.dumps({"timestamp": "2026-07-10T08:05:00.000Z", "type": "event_msg",
                    "payload": {"type": "token_count", "info": {"total_token_usage": {
                        "input_tokens": 5000, "cached_input_tokens": 900,
                        "output_tokens": 700, "total_tokens": 5700}}}}),
    ]
    (day / "rollout-x.jsonl").write_text("\n".join(lines) + "\n")
    u = metrics.brain_usage(claude_dir=tmp_path / "noclaude",
                            codex_dir=tmp_path / "sessions", now=NOW)
    # cumulative counters: last total wins, not the sum
    assert u["codex"]["today"]["input"] == 5000
    assert u["codex"]["today"]["output"] == 700
    assert u["claude"] is None


def test_stats_cli_runs(orc_home, fake_pi):
    from tests.test_runner import run_orc, seed_ok_quota
    seed_ok_quota(orc_home)
    run_orc("run", "hello")
    r = run_orc("stats")
    assert r.returncode == 0
    assert "DELEGATED VALUE" in r.stdout
    r2 = run_orc("stats", "--json")
    assert json.loads(r2.stdout)["workers"]["runs"] == 1
