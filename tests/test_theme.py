import dataclasses

from orc_pkg import registry
from orc_pkg.tui import theme as T


def test_two_named_themes_exist():
    assert {"ember", "phosphor"} <= set(T.THEMES)


def test_no_stock_textual_blue_anywhere():
    for th in T.THEMES.values():
        for f in dataclasses.fields(th):
            assert "#0178d4" not in str(getattr(th, f.name)).lower()


def test_load_theme_respects_config(orc_home):
    registry.home().mkdir(parents=True, exist_ok=True)
    (registry.home() / "config.json").write_text('{"theme": "phosphor"}')
    assert T.load_theme().name == "phosphor"


def test_load_theme_unknown_falls_back(orc_home):
    assert T.load_theme({"theme": "nope"}).name == "ember"


def test_gradient_interpolates():
    stops = ((0.0, "#000000"), (1.0, "#ffffff"))
    assert T.gradient_at(0.5, stops) == "#7f7f7f"
    assert T.gradient_at(-1, stops) == "#000000"
    assert T.gradient_at(2, stops) == "#ffffff"


def test_status_and_brain_lookups():
    th = T.THEMES["ember"]
    assert th.status_color("running") == th.run_running
    assert th.status_color("???") == th.text_dim
    assert th.brain_color("claude") == th.brain_claude
    assert th.brain_color("mystery") == th.text_dim
