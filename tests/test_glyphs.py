from orc_pkg.tui import glyphs as G
from orc_pkg.tui.theme import THEMES

TH = THEMES["ember"]


def test_meter_fill_proportional():
    t = G.meter(50, 20, TH)
    # 10 filled cells at 50%, minus up to 2 replaced by threshold ticks
    assert t.plain.count("█") in (8, 9, 10)
    assert len(t.plain) == 20


def test_meter_has_threshold_ticks():
    assert G.meter(90, 40, TH).plain.count("▏") == 2   # warn + block notches


def test_meter_empty_and_full():
    assert "█" not in G.meter(0, 20, TH).plain
    full = G.meter(100, 20, TH).plain
    assert full.count("█") >= 18


def test_meter_unknown():
    assert "?" in G.meter(None, 20, TH).plain


def test_braille_spark_width_and_range():
    s = G.braille_spark([0, 1, 2, 3, 4, 5, 6, 7], 4)
    assert len(s) == 4
    assert all(0x2800 <= ord(c) <= 0x28FF for c in s)


def test_braille_spark_rising_has_more_dots_at_end():
    s = G.braille_spark([1, 1, 1, 1, 8, 8, 8, 8], 4)
    dots_first = bin(ord(s[0]) - 0x2800).count("1")
    dots_last = bin(ord(s[-1]) - 0x2800).count("1")
    assert dots_last > dots_first


def test_braille_spark_empty():
    assert G.braille_spark([], 5) == "⠀" * 5


def test_braille_spark_pads_left():
    s = G.braille_spark([4.0], 3)          # one sample, right-aligned
    assert len(s) == 3
    assert ord(s[0]) == 0x2800 and ord(s[-1]) > 0x2800


def test_block_spark():
    s = G.block_spark([0, 4, 8], 3)
    assert len(s) == 3
    assert s[2] == "█" and s[0] == " "


def test_fmt_tokens():
    assert G.fmt_tokens(0) == "0"
    assert G.fmt_tokens(999) == "999"
    assert G.fmt_tokens(12421) == "12.4k"
    assert G.fmt_tokens(1_234_567) == "1.2M"
    assert G.fmt_tokens(None) == "0"


def test_fmt_usd():
    assert G.fmt_usd(0.000201) == "$0.0002"
    assert G.fmt_usd(1.234) == "$1.23"
    assert G.fmt_usd(0) == "$0.00"


def test_fmt_dur():
    assert G.fmt_dur(42) == "42s"
    assert G.fmt_dur(192) == "3m12s"
    assert G.fmt_dur(3840) == "1h04m"


def test_status_glyphs_are_distinct():
    glyphs = {G.status_glyph(s) for s in
              ("running", "starting", "done", "failed", "killed", "orphaned")}
    assert len(glyphs) == 6
