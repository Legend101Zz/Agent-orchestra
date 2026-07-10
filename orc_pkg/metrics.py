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


def _parse_claude_file(path: Path) -> dict:
    """One session file -> {"days": {day: {...}}, "models": {model: total}}."""
    days: dict = {}
    models: dict = {}
    seen: set = set()
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        for line in f:
            try:
                rec = json.loads(line)
            except ValueError:
                continue
            if not isinstance(rec, dict):
                continue
            msg = rec.get("message")
            if not isinstance(msg, dict):
                continue
            usage = msg.get("usage")
            if not isinstance(usage, dict):
                continue
            key = (msg.get("id"), rec.get("requestId"))
            if key != (None, None):
                if key in seen:
                    continue
                seen.add(key)
            day = str(rec.get("timestamp", ""))[:10]
            d = days.setdefault(day, {"input": 0, "output": 0,
                                      "cache_read": 0, "cache_create": 0})
            inp = int(usage.get("input_tokens", 0) or 0)
            outp = int(usage.get("output_tokens", 0) or 0)
            d["input"] += inp
            d["output"] += outp
            d["cache_read"] += int(usage.get("cache_read_input_tokens", 0) or 0)
            d["cache_create"] += int(usage.get("cache_creation_input_tokens", 0) or 0)
            model = msg.get("model") or "?"
            models[model] = models.get(model, 0) + inp + outp
    return {"days": days, "models": models}


def _parse_codex_file(path: Path) -> dict:
    """Codex token_count carries *cumulative* totals — the last one per file wins."""
    last = None
    last_day = None
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        for line in f:
            try:
                rec = json.loads(line)
            except ValueError:
                continue
            if not isinstance(rec, dict):
                continue
            payload = rec.get("payload")
            if not isinstance(payload, dict) or payload.get("type") != "token_count":
                continue
            info = payload.get("info")
            if not isinstance(info, dict):
                continue
            total = info.get("total_token_usage") or info.get("last_token_usage")
            if isinstance(total, dict):
                last = total
                last_day = str(rec.get("timestamp", ""))[:10]
    if not last:
        return {"days": {}, "models": {}}
    if not last_day:
        last_day = datetime.fromtimestamp(
            path.stat().st_mtime, tz=timezone.utc).strftime("%Y-%m-%d")
    return {"days": {last_day: {
        "input": int(last.get("input_tokens", 0) or 0),
        "output": int(last.get("output_tokens", 0) or 0),
        "cache_read": int(last.get("cached_input_tokens", 0) or 0),
        "cache_create": 0,
    }}, "models": {}}


_ZERO = {"input": 0, "output": 0, "cache_read": 0, "cache_create": 0}


def _combine(files: dict, now: datetime) -> dict | None:
    if not files:
        return None
    today_key = now.strftime("%Y-%m-%d")
    week_floor = (now - timedelta(days=7)).strftime("%Y-%m-%d")
    today = dict(_ZERO)
    week = dict(_ZERO)
    by_model: dict = {}
    for agg in files.values():
        for day, d in agg.get("days", {}).items():
            if day == today_key:
                for k in today:
                    today[k] += d.get(k, 0)
            if day >= week_floor:
                for k in week:
                    week[k] += d.get(k, 0)
        for model, tok in agg.get("models", {}).items():
            by_model[model] = by_model.get(model, 0) + tok
    return {"today": today, "week": week, "by_model": by_model}


def brain_usage(claude_dir=None, codex_dir=None, cache_path=None, now=None) -> dict:
    """Brain-side token usage parsed from local session logs, mtime-cached.

    Returns {"claude": {...} | None, "codex": {...} | None}; None means the
    logs are absent/unparseable — render as "n/a", never crash.
    """
    claude_dir = Path(claude_dir) if claude_dir else Path.home() / ".claude" / "projects"
    codex_dir = Path(codex_dir) if codex_dir else Path.home() / ".codex" / "sessions"
    cache_path = Path(cache_path) if cache_path else registry.home() / "brain_usage_cache.json"
    now = now or datetime.now(timezone.utc)

    try:
        with open(cache_path, "r", encoding="utf-8") as f:
            cache = json.load(f)
        if not isinstance(cache.get("files"), dict):
            cache = {"files": {}}
    except (OSError, ValueError):
        cache = {"files": {}}

    live_paths: set = set()
    result: dict = {}
    dirty = False
    for name, root, parser in (("claude", claude_dir, _parse_claude_file),
                               ("codex", codex_dir, _parse_codex_file)):
        try:
            paths = sorted(root.glob("**/*.jsonl")) if root.is_dir() else []
        except OSError:
            paths = []
        files: dict = {}
        for p in paths:
            sp = str(p)
            live_paths.add(sp)
            try:
                st = p.stat()
            except OSError:
                continue
            cached = cache["files"].get(sp)
            if (isinstance(cached, dict) and cached.get("mtime") == st.st_mtime
                    and cached.get("size") == st.st_size):
                files[sp] = cached.get("agg", {})
                continue
            try:
                agg = parser(p)
            except Exception:
                continue
            files[sp] = agg
            cache["files"][sp] = {"mtime": st.st_mtime, "size": st.st_size, "agg": agg}
            dirty = True
        result[name] = _combine(files, now)

    stale = [sp for sp in cache["files"] if sp not in live_paths]
    for sp in stale:
        del cache["files"][sp]
        dirty = True
    if dirty:
        try:
            registry.atomic_write_json(cache_path, cache)
        except OSError:
            pass
    return result


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
