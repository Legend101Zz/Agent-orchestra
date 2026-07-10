import json

from orc_pkg.tui.convo import parse_log


def evt(t, delta):
    return json.dumps({"type": "message_update",
                       "assistantMessageEvent": {"type": t, "delta": delta}})


def test_parse_json_log(tmp_path):
    p = tmp_path / "output.log"
    p.write_text(evt("thinking_delta", "hmm ") + "\n" + evt("text_delta", "Hello ")
                 + "\n" + evt("text_delta", "world") + "\n"
                 + '{"type":"agent_end","messages":[]}\n')
    r = parse_log(p)
    assert r["reply"] == "Hello world"
    assert r["thinking"] == "hmm "
    assert r["plain"] is None
    assert r["events"] == 4


def test_parse_plain_log(tmp_path):
    p = tmp_path / "output.log"
    p.write_text("just plain text\nfrom old pi -p mode\n")
    r = parse_log(p)
    assert r["plain"].startswith("just plain")
    assert r["reply"] == ""


def test_parse_mixed_mostly_plain(tmp_path):
    p = tmp_path / "output.log"
    p.write_text("line a\nline b\nline c\n" + evt("text_delta", "x") + "\n")
    r = parse_log(p)
    assert r["plain"] is not None      # <50% JSON → legacy treatment


def test_parse_missing(tmp_path):
    r = parse_log(tmp_path / "nope.log")
    assert r == {"reply": "", "thinking": "", "plain": None, "events": 0}
