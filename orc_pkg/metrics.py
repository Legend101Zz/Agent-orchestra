"""Aggregate worker registry usage and brain-side (Claude/Codex) session logs.

Worker numbers come from the registry and are *exact* when pi's agent_end usage
was recorded (`tokens.total` present), *estimated* (chars/4) otherwise. Brain
numbers are parsed from local session logs and are always labeled
API-equivalent — subscriptions are flat-rate, nobody pays these per-token.
"""

from __future__ import annotations

import json
import os
from datetime import datetime, timedelta, timezone
from pathlib import Path

from orc_pkg import registry

# USD per 1M tokens (input, output).
WORKER_PRICE = (0.30, 1.20)          # MiniMax-M3 list price
BRAIN_PRICE = {                       # API-equivalent list prices per brain
    "claude": (3.0, 15.0),            # Claude Sonnet class
    "codex": (1.25, 10.0),            # GPT-5 class
    "human": (3.0, 15.0),             # priced as if a brain had done it
}


def _is_exact(tokens: dict) -> bool:
    return bool(tokens.get("total"))


def _run_cost_usd(tokens: dict) -> float:
    """Exact recorded cost, else list-price the exact split, else 0."""
    if tokens.get("cost_usd") is not None:
        return float(tokens["cost_usd"])
    if _is_exact(tokens):
        return (tokens.get("input", 0) / 1e6 * WORKER_PRICE[0]
                + tokens.get("output", 0) / 1e6 * WORKER_PRICE[1])
    return 0.0


_STATUS_RANK = {"running": 0, "starting": 1, "failed": 2, "killed": 3,
                "orphaned": 4, "done": 5}


def worst_status(statuses) -> str:
    """The most attention-worthy status in a group (running > failed > ... > done)."""
    ranked = [s for s in statuses if s in _STATUS_RANK]
    if not ranked:
        return "done"
    return min(ranked, key=lambda s: _STATUS_RANK[s])


def worker_stats(runs: list) -> dict:
    out = {
        "runs": 0,
        "by_status": {},
        "exact": {"runs": 0, "input": 0, "output": 0, "cache_read": 0,
                  "total": 0, "cost_usd": 0.0},
        "estimated": {"runs": 0, "total": 0},
        "by_brain": {},
        "by_session": {},
        "by_day": {},
    }
    session_statuses: dict = {}
    for m in runs:
        if not isinstance(m, dict):
            continue
        out["runs"] += 1
        status = m.get("status", "?")
        out["by_status"][status] = out["by_status"].get(status, 0) + 1
        tokens = m.get("tokens") or {}
        exact = _is_exact(tokens)
        cost = _run_cost_usd(tokens)
        total = tokens.get("total") if exact else tokens.get("estimated_total", 0)
        total = int(total or 0)

        if exact:
            e = out["exact"]
            e["runs"] += 1
            e["input"] += int(tokens.get("input", 0) or 0)
            e["output"] += int(tokens.get("output", 0) or 0)
            e["cache_read"] += int(tokens.get("cache_read", 0) or 0)
            e["total"] += total
            e["cost_usd"] = round(e["cost_usd"] + cost, 6)
        else:
            out["estimated"]["runs"] += 1
            out["estimated"]["total"] += total

        brain = m.get("brain", "human")
        b = out["by_brain"].setdefault(brain, {"runs": 0, "total": 0, "cost_usd": 0.0})
        b["runs"] += 1
        b["total"] += total
        b["cost_usd"] = round(b["cost_usd"] + cost, 6)

        sess = m.get("session")
        if sess:
            s = out["by_session"].setdefault(
                sess, {"runs": 0, "total": 0, "cost_usd": 0.0, "status": "done"})
            s["runs"] += 1
            s["total"] += total
            s["cost_usd"] = round(s["cost_usd"] + cost, 6)
            session_statuses.setdefault(sess, []).append(status)

        day = str(m.get("started_at", ""))[:10]
        if day:
            d = out["by_day"].setdefault(day, {"runs": 0, "total": 0})
            d["runs"] += 1
            d["total"] += total

    for sess, statuses in session_statuses.items():
        out["by_session"][sess]["status"] = worst_status(statuses)
    return out


def brain_usage(claude_dir=None, codex_dir=None, cache_path=None, now=None) -> dict:
    """Brain-side token usage parsed from local session logs (see Task 5)."""
    return {"claude": None, "codex": None}


def delegated_value(runs: list) -> dict:
    """The hero number: what these worker tokens would have cost at brain prices."""
    worker_cost = 0.0
    brain_equiv = 0.0
    exact_tokens = 0
    all_tokens = 0
    for m in runs:
        if not isinstance(m, dict):
            continue
        tokens = m.get("tokens") or {}
        rate = BRAIN_PRICE.get(m.get("brain", "human"), BRAIN_PRICE["human"])
        worker_cost += _run_cost_usd(tokens)
        if _is_exact(tokens):
            inp, outp = tokens.get("input", 0) or 0, tokens.get("output", 0) or 0
            brain_equiv += inp / 1e6 * rate[0] + outp / 1e6 * rate[1]
            exact_tokens += int(tokens.get("total", 0) or 0)
            all_tokens += int(tokens.get("total", 0) or 0)
        else:
            est = int(tokens.get("estimated_total", 0) or 0)
            brain_equiv += est / 1e6 * rate[0]      # conservatively all-input
            worker_cost += est / 1e6 * WORKER_PRICE[0]
            all_tokens += est

    saved = round(brain_equiv - worker_cost, 4)
    return {
        "worker_cost_usd": round(worker_cost, 6),
        "brain_equiv_usd": round(brain_equiv, 6),
        "saved_usd": saved if all_tokens else 0,
        "multiple": round(brain_equiv / worker_cost, 1) if worker_cost > 0 else 0,
        "exact_share": round(exact_tokens / all_tokens, 3) if all_tokens else 0.0,
    }
