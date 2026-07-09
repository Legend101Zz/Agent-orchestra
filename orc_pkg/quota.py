"""MiniMax coding-plan quota: fetch, parse, threshold levels, cache."""
import json
import os
import subprocess
import time
import urllib.request
from pathlib import Path

from orc_pkg import registry

REMAINS_URL = "https://api.minimax.io/v1/token_plan/remains"

DEFAULT_CONFIG = {
    "warn_pct": 25,
    "block_pct": 10,
    "cache_ttl_sec": 60,
    "max_parallel_workers": 3,
}


def load_config() -> dict:
    cfg = dict(DEFAULT_CONFIG)
    try:
        path = registry.home() / "config.json"
        if path.exists():
            with open(path, "r", encoding="utf-8") as f:
                overlay = json.load(f)
            if isinstance(overlay, dict):
                cfg.update(overlay)
    except (ValueError, OSError):
        pass
    return cfg


def get_key() -> str | None:
    try:
        result = subprocess.run(
            [
                "security",
                "find-generic-password",
                "-a",
                os.environ.get("USER", ""),
                "-s",
                "minimax_api_key",
                "-w",
            ],
            capture_output=True,
            text=True,
            timeout=10,
        )
        if result.returncode == 0:
            key = result.stdout.strip()
            if key:
                return key
    except Exception:
        pass
    try:
        auth_path = Path.home() / ".pi" / "agent" / "auth.json"
        with open(auth_path, "r", encoding="utf-8") as f:
            data = json.load(f)
        entry = data.get("minimax")
        if isinstance(entry, dict):
            return entry.get("key") or entry.get("apiKey")
    except Exception:
        return None
    return None


def fetch_remains(key) -> dict:
    req = urllib.request.Request(
        REMAINS_URL,
        headers={
            "Authorization": f"Bearer {key}",
            "Content-Type": "application/json",
        },
    )
    with urllib.request.urlopen(req, timeout=15) as resp:
        body = resp.read().decode("utf-8")
    return json.loads(body)


def parse_remains(raw) -> dict | None:
    entries = raw.get("model_remains") or []
    for entry in entries:
        if entry.get("model_name") == "general":
            return {
                "five_hour_pct": entry["current_interval_remaining_percent"],
                "weekly_pct": entry["current_weekly_remaining_percent"],
                "window_resets_in_min": round(entry.get("remains_time", 0) / 60000),
                "fetched_at": time.time(),
            }
    return None


def level_for(parsed, cfg) -> str:
    pct = min(parsed["five_hour_pct"], parsed["weekly_pct"])
    if pct <= cfg["block_pct"]:
        return "block"
    if pct <= cfg["warn_pct"]:
        return "warn"
    return "ok"


def get_quota(force: bool = False) -> dict:
    cfg = load_config()
    home = registry.home()
    home.mkdir(parents=True, exist_ok=True)
    cache_path = home / "quota.json"

    if not force:
        try:
            with open(cache_path, "r", encoding="utf-8") as f:
                cached = json.load(f)
            if time.time() - cached.get("fetched_at", 0) < cfg["cache_ttl_sec"]:
                cached["level"] = level_for(cached, cfg)
                cached["source"] = "cache"
                return cached
        except (ValueError, KeyError, TypeError, OSError):
            pass

    key = get_key()
    if not key:
        return {"level": "unknown", "reason": "no MiniMax key in Keychain or auth.json"}

    try:
        parsed = parse_remains(fetch_remains(key))
    except Exception as e:
        return {"level": "unknown", "reason": str(e)}

    if parsed is None:
        return {
            "level": "unknown",
            "reason": "no 'general' entry — key may not be a coding-plan key",
        }

    registry.atomic_write_json(cache_path, parsed)
    result = dict(parsed)
    result["level"] = level_for(parsed, cfg)
    result["source"] = "api"
    return result
