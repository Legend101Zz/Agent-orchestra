from orc_pkg.tui.flow import render_flow
from orc_pkg.tui.theme import THEMES

TH = THEMES["ember"]


def R(i, status="done", brain="claude", tokens=110000):
    return {"id": i, "status": status, "brain": brain, "task": f"task {i}",
            "started_at": "2026-07-10T10:00:00+00:00",
            "ended_at": "2026-07-10T10:03:12+00:00",
            "tokens": {"total": tokens, "estimated_total": tokens,
                       "cost_usd": 0.042}}


def test_flow_renders_brain_and_workers():
    runs = [R("run-aaa"), R("run-bbb", status="running")]
    out = render_flow(runs, TH).plain
    assert "claude" in out
    assert "run-aaa" in out and "run-bbb" in out
    assert "◉" in out and "●" in out        # running + done glyphs
    assert "─" in out and "│" in out         # connectors and box edges
    assert "110.0k" in out                   # per-node tokens


def test_flow_two_brains_two_rails():
    runs = [R("x1"), R("x2", brain="codex")]
    out = render_flow(runs, TH).plain
    assert "claude" in out and "codex" in out


def test_flow_single_worker_direct_line():
    out = render_flow([R("solo")], TH).plain
    assert "solo" in out


def test_flow_legacy_meta_no_tokens():
    out = render_flow([{"id": "old-run", "status": "done", "brain": "human"}], TH).plain
    assert "old-run" in out


def test_flow_empty_placeholder():
    assert render_flow([], TH).plain.strip() != ""
