import pytest

from tests.test_runner import seed_ok_quota

from orc_pkg import registry


@pytest.fixture
def some_runs(orc_home):
    seed_ok_quota(orc_home)
    rd = registry.new_run("visible task one", brain="claude")
    m = registry.read_meta(rd)
    m["status"] = "done"
    registry.write_meta(rd, m)
    return orc_home


async def test_tui_smoke_renders_runs_and_quota(some_runs, monkeypatch):
    from orc_pkg import quota
    monkeypatch.setattr(quota, "get_quota", lambda force=False: {
        "level": "ok", "five_hour_pct": 83, "weekly_pct": 49,
        "window_resets_in_min": 32, "source": "cache"})
    from orc_pkg.tui import OrcTop
    app = OrcTop()
    async with app.run_test(size=(120, 40)) as pilot:
        await pilot.pause()
        table = app.query_one("#runs-table")
        assert table.row_count == 1
        assert app.query_one("#quota-panel") is not None
        await pilot.press("q")
