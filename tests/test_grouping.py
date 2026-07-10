from orc_pkg.tui.grouping import flatten, group_sessions


def R(i, sess=None, status="done", ts=0, tokens=1000, brain="claude"):
    m = {"id": i, "status": status, "created_ts": ts, "brain": brain,
         "task": f"task {i}", "started_at": "2026-07-10T00:00:00+00:00",
         "tokens": {"estimated_total": tokens, "total": tokens, "cost_usd": 0.01}}
    if sess:
        m["session"] = sess
    return m


def test_groups_sessions_and_leaves_singles():
    rows = group_sessions([R("a", "s1", ts=3), R("b", "s1", ts=2), R("c", ts=1)])
    assert [r["kind"] for r in rows] == ["session", "run"]
    assert rows[0]["n"] == 2 and rows[0]["tokens"] == 2000
    assert rows[0]["cost_usd"] == 0.02
    assert rows[0]["brains"] == {"claude"}


def test_running_bubbles_to_top():
    rows = group_sessions([R("new", ts=9), R("old", "s1", status="running", ts=1)])
    assert rows[0]["kind"] == "session"


def test_session_status_is_worst():
    rows = group_sessions([R("a", "s1", status="done"), R("b", "s1", status="failed")])
    assert rows[0]["status"] == "failed"


def test_old_meta_without_tokens_still_groups():
    m = {"id": "legacy", "status": "done", "brain": "human"}   # pre-v2 meta
    rows = group_sessions([m])
    assert rows[0]["kind"] == "run"


def test_flatten_expansion():
    g = group_sessions([R("a", "s1", ts=2), R("b", "s1", ts=1), R("c", ts=0)])
    vis = flatten(g, expanded=set())
    assert [v["kind"] for v in vis] == ["session", "run"]
    vis = flatten(g, expanded={"s1"})
    assert [v["kind"] for v in vis] == ["session", "member", "member", "run"]
    assert vis[1]["last"] is False and vis[2]["last"] is True
