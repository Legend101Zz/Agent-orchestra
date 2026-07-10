"""Reconstruct the worker conversation from a run's event log.

New runs log raw pi JSONL events; old runs logged plain text. If fewer than
half of a log's non-empty lines parse as JSON events, we treat the whole file
as legacy plain output rather than half-rendering it. Never raises.
"""

from __future__ import annotations

import json
from pathlib import Path

_EMPTY = {"reply": "", "thinking": "", "plain": None, "events": 0}


def parse_log(log_path) -> dict:
    try:
        text = Path(log_path).read_text(errors="replace")
    except OSError:
        return dict(_EMPTY)

    lines = [ln for ln in text.splitlines() if ln.strip()]
    if not lines:
        return dict(_EMPTY)

    reply: list = []
    thinking: list = []
    events = 0
    for ln in lines:
        try:
            evt = json.loads(ln)
        except ValueError:
            continue
        if not isinstance(evt, dict) or "type" not in evt:
            continue
        events += 1
        ame = evt.get("assistantMessageEvent")
        if not isinstance(ame, dict):
            continue
        delta = ame.get("delta")
        if not isinstance(delta, str):
            continue
        kind = ame.get("type", "")
        if kind == "text_delta":
            reply.append(delta)
        elif "thinking" in kind:
            thinking.append(delta)

    if events * 2 < len(lines):
        return {"reply": "", "thinking": "", "plain": text, "events": events}
    return {"reply": "".join(reply), "thinking": "".join(thinking),
            "plain": None, "events": events}
