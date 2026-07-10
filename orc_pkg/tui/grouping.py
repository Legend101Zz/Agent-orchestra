"""Session tree model for the dashboard table: group, order, flatten."""

from __future__ import annotations

from orc_pkg.metrics import worst_status

_ACTIVE = ("running", "starting")


def _tok(m: dict) -> int:
    tokens = m.get("tokens") or {}
    return int(tokens.get("total") or tokens.get("estimated_total") or 0)


def _cost(m: dict) -> float:
    tokens = m.get("tokens") or {}
    return float(tokens.get("cost_usd") or 0)


def group_sessions(runs: list) -> list:
    """Top-level rows: sessions (grouped) + standalone runs, active first."""
    sessions: dict = {}
    order: list = []
    for m in runs:
        if not isinstance(m, dict) or not m.get("id"):
            continue
        sess = m.get("session")
        if not sess:
            order.append({"kind": "run", "meta": m})
            continue
        g = sessions.get(sess)
        if g is None:
            g = {"kind": "session", "key": sess, "runs": [], "n": 0,
                 "status": "done", "tokens": 0, "cost_usd": 0.0,
                 "started_ts": 0.0, "brains": set()}
            sessions[sess] = g
            order.append(g)
        g["runs"].append(m)
        g["n"] += 1
        g["tokens"] += _tok(m)
        g["cost_usd"] = round(g["cost_usd"] + _cost(m), 6)
        g["started_ts"] = max(g["started_ts"], float(m.get("created_ts") or 0))
        g["brains"].add(m.get("brain", "human"))

    for g in sessions.values():
        g["status"] = worst_status(r.get("status") for r in g["runs"])

    def sort_key(row):
        if row["kind"] == "session":
            active = row["status"] in _ACTIVE
            ts = row["started_ts"]
        else:
            active = row["meta"].get("status") in _ACTIVE
            ts = float(row["meta"].get("created_ts") or 0)
        return (not active, -ts)

    order.sort(key=sort_key)
    return order


def flatten(groups: list, expanded: set) -> list:
    """Visible rows for the table given the set of expanded session keys."""
    out: list = []
    for row in groups:
        out.append(row)
        if row["kind"] == "session" and row["key"] in expanded:
            members = row["runs"]
            for i, m in enumerate(members):
                out.append({"kind": "member", "meta": m,
                            "last": i == len(members) - 1})
    return out
