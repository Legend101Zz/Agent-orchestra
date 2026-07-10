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


def test_stats_cli_runs(orc_home, fake_pi):
    from tests.test_runner import run_orc, seed_ok_quota
    seed_ok_quota(orc_home)
    run_orc("run", "hello")
    r = run_orc("stats")
    assert r.returncode == 0
    assert "DELEGATED VALUE" in r.stdout
    r2 = run_orc("stats", "--json")
    assert json.loads(r2.stdout)["workers"]["runs"] == 1
