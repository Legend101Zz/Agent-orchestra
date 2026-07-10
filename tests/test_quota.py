import json
import time

from orc_pkg import quota, registry

# Captured live from api.minimax.io on 2026-07-10 (schema exact)
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


def test_history_append_and_read(orc_home):
    quota.append_history({"five_hour_pct": 80, "weekly_pct": 60})
    quota.append_history({"five_hour_pct": 78, "weekly_pct": 59})
    hist = quota.read_history()
    assert [h["five_hour_pct"] for h in hist] == [80, 78]
    assert all("ts" in h for h in hist)


def test_history_tolerates_garbage(orc_home):
    p = registry.home() / "quota_history.jsonl"
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text('not json\n{"ts": 1, "five_hour_pct": 50, "weekly_pct": 40}\n')
    assert quota.read_history() == [{"ts": 1, "five_hour_pct": 50, "weekly_pct": 40}]


def test_get_quota_api_fetch_appends_history(orc_home, monkeypatch):
    monkeypatch.setattr(quota, "fetch_remains", lambda key: RAW)
    monkeypatch.setattr(quota, "get_key", lambda: "k")
    quota.get_quota(force=True)
    hist = quota.read_history()
    assert len(hist) == 1 and hist[0]["five_hour_pct"] == 83
