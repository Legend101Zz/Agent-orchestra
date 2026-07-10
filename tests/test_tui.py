import pytest

from tests.test_runner import seed_ok_quota

from orc_pkg import registry


@pytest.fixture
def quiet_backends(monkeypatch):
    """Deterministic quota + brain data so TUI tests never touch network/disk."""
    from orc_pkg import metrics, quota
    monkeypatch.setattr(quota, "get_quota", lambda force=False: {
        "level": "ok", "five_hour_pct": 83, "weekly_pct": 49,
        "window_resets_in_min": 32, "source": "cache"})
    monkeypatch.setattr(quota, "read_history", lambda limit=96: [
        {"ts": 1, "five_hour_pct": 90, "weekly_pct": 60},
        {"ts": 2, "five_hour_pct": 83, "weekly_pct": 49}])
    monkeypatch.setattr(metrics, "brain_usage",
                        lambda *a, **kw: {"claude": None, "codex": None})


@pytest.fixture
def some_runs(orc_home):
    seed_ok_quota(orc_home)
    rd = registry.new_run("visible task one", brain="claude")
    m = registry.read_meta(rd)
    m["status"] = "done"
    registry.write_meta(rd, m)
    return orc_home


@pytest.fixture
def session_runs(orc_home):
    seed_ok_quota(orc_home)
    rd = registry.new_run("standalone run", brain="human")
    m = registry.read_meta(rd)
    m["status"] = "done"
    registry.write_meta(rd, m)
    for i, status in enumerate(("done", "running")):
        rd = registry.new_run(f"swarm chunk {i}", brain="claude", session="orch-x")
        m = registry.read_meta(rd)
        m["status"] = status
        m["created_ts"] = m["created_ts"] + 10 + i     # session is newest
        if status == "done":
            m["tokens"] = {"input": 1000, "output": 100, "total": 1100,
                           "cost_usd": 0.001, "estimated_total": 1100}
        registry.write_meta(rd, m)
    return orc_home


async def test_tui_smoke_renders_runs_and_quota(some_runs, quiet_backends):
    from orc_pkg.tui import OrcTop
    app = OrcTop()
    async with app.run_test(size=(120, 40)) as pilot:
        await pilot.pause()
        table = app.query_one("#runs-table")
        assert table.row_count == 1
        assert app.query_one("#quota-panel") is not None
        await pilot.press("q")


async def test_dashboard_renders_tiles_and_meters(some_runs, quiet_backends):
    from orc_pkg.tui import OrcTop
    app = OrcTop()
    async with app.run_test(size=(140, 44)) as pilot:
        await pilot.pause()
        await pilot.pause()
        for tid in ("#tile-value", "#tile-tokens", "#tile-cost", "#tile-active",
                    "#activity", "#hdr", "#ftr"):
            assert app.query_one(tid) is not None
        assert app.quota_state["five_hour_pct"] == 83
        await pilot.press("q")


async def test_dashboard_session_expands(session_runs, quiet_backends):
    from orc_pkg.tui import OrcTop
    app = OrcTop()
    async with app.run_test(size=(140, 44)) as pilot:
        await pilot.pause()
        table = app.query_one("#runs-table")
        assert table.row_count == 2            # collapsed session + standalone
        await pilot.press("enter")             # expand first (session) row
        await pilot.pause()
        assert table.row_count == 4
        await pilot.press("enter")             # collapse again
        await pilot.pause()
        assert table.row_count == 2
        await pilot.press("q")


async def test_dashboard_filter_narrows(session_runs, quiet_backends):
    from orc_pkg.tui import OrcTop
    app = OrcTop()
    async with app.run_test(size=(140, 44)) as pilot:
        await pilot.pause()
        app.filter_text = "standalone"
        app.refresh_data()
        await pilot.pause()
        assert app.query_one("#runs-table").row_count == 1
        await pilot.press("q")


async def test_theme_never_stock_blue(some_runs, quiet_backends):
    from orc_pkg.tui import OrcTop
    app = OrcTop()
    async with app.run_test(size=(140, 44)) as pilot:
        await pilot.pause()
        assert "#0178d4" not in app.export_screenshot().lower()
        await pilot.press("q")


async def test_enter_run_opens_session_screen(session_runs, quiet_backends):
    from orc_pkg.tui import OrcTop
    from orc_pkg.tui.session_screen import SessionScreen
    app = OrcTop()
    async with app.run_test(size=(140, 44)) as pilot:
        await pilot.pause()
        await pilot.press("enter")             # expand session
        await pilot.pause()
        await pilot.press("j", "enter")        # first member → drill in
        await pilot.pause()
        assert isinstance(app.screen, SessionScreen)
        assert len(app.screen.runs) == 2       # whole session opened
        await pilot.press("escape")
        await pilot.pause()
        assert not isinstance(app.screen, SessionScreen)
        await pilot.press("q")


async def test_session_screen_has_four_tabs(session_runs, quiet_backends):
    from textual.widgets import TabbedContent, TabPane
    from orc_pkg.tui import OrcTop
    from orc_pkg.tui.session_screen import SessionScreen
    app = OrcTop()
    async with app.run_test(size=(140, 44)) as pilot:
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
        await pilot.press("j", "enter")
        await pilot.pause()
        assert isinstance(app.screen, SessionScreen)
        panes = [p.id for p in app.screen.query(TabPane)]
        assert panes == ["tab-flow", "tab-convo", "tab-log", "tab-meta"]
        await pilot.press("escape")
        await pilot.press("q")


async def test_session_screen_cycles_selected_run(session_runs, quiet_backends):
    from orc_pkg.tui import OrcTop
    from orc_pkg.tui.session_screen import SessionScreen
    app = OrcTop()
    async with app.run_test(size=(140, 44)) as pilot:
        await pilot.pause()
        await pilot.press("enter")
        await pilot.pause()
        await pilot.press("j", "enter")
        await pilot.pause()
        scr = app.screen
        assert isinstance(scr, SessionScreen)
        before = scr.current["id"]
        await pilot.press("right_square_bracket")
        assert scr.current["id"] != before
        await pilot.press("escape")
        await pilot.press("q")


async def test_help_overlay_toggles(some_runs, quiet_backends):
    from orc_pkg.tui import OrcTop
    app = OrcTop()
    async with app.run_test(size=(140, 44)) as pilot:
        await pilot.pause()
        assert not app.query_one("#help-wrap").has_class("visible")
        await pilot.press("question_mark")
        assert app.query_one("#help-wrap").has_class("visible")
        await pilot.press("j")                 # any key closes
        assert not app.query_one("#help-wrap").has_class("visible")
        await pilot.press("q")
