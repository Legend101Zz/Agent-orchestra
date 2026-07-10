"""Spawn pi workers: foreground/background runs with quota gating and idle watchdog."""
import json
import os
import selectors
import signal
import subprocess
import sys
import time
from pathlib import Path

from orc_pkg import quota, registry

PI_BASE = [
    "pi",
    "-p",
    "--mode",
    "json",
    "--offline",
    "--provider",
    "minimax",
    "--model",
    "MiniMax-M3",
    "--no-session",
]


def quota_gate(force: bool) -> bool:
    q = quota.get_quota()
    level = q.get("level", "unknown")
    if level == "warn":
        print(
            f"ORC WARNING: MiniMax quota low — 5h window {q['five_hour_pct']}% "
            f"/ weekly {q['weekly_pct']}% remaining. Consider pausing delegation.",
            file=sys.stderr,
        )
        return True
    if level == "block":
        print(
            f"ORC BLOCKED: MiniMax quota below block threshold "
            f"(5h {q['five_hour_pct']}%, weekly {q['weekly_pct']}%). "
            f"Use --force to override.",
            file=sys.stderr,
        )
        return force
    if level == "unknown":
        print(
            f"ORC NOTE: quota unknown ({q.get('reason', '')}) — proceeding.",
            file=sys.stderr,
        )
        return True
    return True


def finalize(rd: Path, meta: dict, code: int, usage: dict | None = None) -> None:
    if code == 0:
        status = "done"
    elif code < 0:
        status = "killed"
    else:
        status = "failed"
    meta["status"] = status
    meta["exit_code"] = code
    meta["ended_at"] = registry.now_iso()
    task = meta.get("task", "")
    log_path = rd / "output.log"
    out_size = 0
    if log_path.exists():
        try:
            out_size = log_path.stat().st_size
        except OSError:
            out_size = 0
    tokens = meta.setdefault("tokens", {})
    tokens["estimated_total"] = (len(task) + out_size) // 4
    if usage:
        tokens.update(usage)
        tokens["estimated_total"] = usage.get("total", tokens["estimated_total"])
    registry.write_meta(rd, meta)


def _exec(rd: Path, echo: bool = False, idle_timeout: float | None = None) -> int:
    meta = registry.read_meta(rd)
    if idle_timeout is None:
        idle_timeout = float(quota.load_config().get("idle_timeout_sec", 300))

    log_file = open(rd / "output.log", "ab")
    code = 0
    buf = b""
    usage: dict | None = None
    echoed_delta = False

    def handle_line(line: bytes) -> None:
        # Raw event line goes to the log; only extracted text (or non-JSON
        # passthrough) reaches the caller's stdout.
        nonlocal usage, echoed_delta
        log_file.write(line + b"\n")
        log_file.flush()
        try:
            evt = json.loads(line)
        except ValueError:
            evt = None
        if not isinstance(evt, dict):
            if echo and line:
                sys.stdout.buffer.write(line + b"\n")
                sys.stdout.buffer.flush()
            return
        text = _extract_text(evt)
        if text and echo:
            sys.stdout.write(text)
            sys.stdout.flush()
            echoed_delta = True
        if evt.get("type") in TERMINAL_EVENTS:
            usage = _extract_usage(evt) or usage

    try:
        try:
            proc = subprocess.Popen(
                PI_BASE + [meta["task"]],
                cwd=meta["cwd"],
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
        except FileNotFoundError:
            log_file.write(b"orc: pi executable not found on PATH\n")
            log_file.flush()
            if echo:
                sys.stderr.buffer.write(b"orc: pi executable not found on PATH\n")
                sys.stderr.buffer.flush()
            finalize(rd, meta, 127)
            return 127

        meta["pid"] = proc.pid
        meta["status"] = "running"
        registry.write_meta(rd, meta)

        sel = selectors.DefaultSelector()
        sel.register(proc.stdout, selectors.EVENT_READ)
        last_output = time.monotonic()

        try:
            try:
                while True:
                    events = sel.select(timeout=0.5)
                    if not events:
                        if proc.poll() is not None:
                            break
                        if idle_timeout > 0 and (time.monotonic() - last_output) > idle_timeout:
                            msg = (
                                f"\norc: idle timeout after {int(idle_timeout)}s — killing worker\n"
                            ).encode()
                            log_file.write(msg)
                            log_file.flush()
                            if echo:
                                sys.stderr.buffer.write(msg)
                                sys.stderr.buffer.flush()
                            try:
                                os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
                            except ProcessLookupError:
                                pass
                            proc.wait()
                            finalize(rd, meta, 124, usage)
                            return 124
                        continue
                    chunk = os.read(proc.stdout.fileno(), 65536)
                    if not chunk:
                        break
                    buf += chunk
                    while b"\n" in buf:
                        line, buf = buf.split(b"\n", 1)
                        handle_line(line)
                    last_output = time.monotonic()
            except KeyboardInterrupt:
                try:
                    os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
                except ProcessLookupError:
                    pass
                code = proc.wait()
                if code >= 0:
                    code = -signal.SIGTERM
            else:
                code = proc.wait()
        finally:
            if buf:
                try:
                    handle_line(buf)
                except Exception:
                    pass
                buf = b""
            try:
                sel.close()
            except Exception:
                pass
            try:
                proc.stdout.close()
            except Exception:
                pass
    finally:
        log_file.close()

    if echoed_delta:
        sys.stdout.write("\n")
        sys.stdout.flush()
    # pi traps SIGTERM and exits 143 (128+15) instead of dying by signal; an
    # inbox kill marker or that code means "killed by orc", not "failed".
    if code > 0 and (code == 143 or _inbox_has_kill(rd)):
        code = -signal.SIGTERM
    finalize(rd, meta, code, usage)
    if code >= 0:
        return max(code, 0)
    return 130


RPC_BASE = [
    "pi",
    "--mode",
    "rpc",
    "--offline",
    "--provider",
    "minimax",
    "--model",
    "MiniMax-M3",
    "--no-session",
]
# Real pi rpc protocol (verified live 2026-07-10): assistant text arrives as
# {"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":...}}
# and the run terminates with {"type":"agent_end",...}. pi exits when stdin closes,
# so stdin must stay open until agent_end.
TERMINAL_EVENTS = {"agent_end"}


def _extract_text(evt: dict):
    ame = evt.get("assistantMessageEvent")
    if isinstance(ame, dict) and ame.get("type") == "text_delta":
        delta = ame.get("delta")
        if isinstance(delta, str):
            return delta
    return None


def _inbox_has_kill(rd: Path) -> bool:
    inbox = rd / "inbox"
    return inbox.is_dir() and any(inbox.glob("kill-*.json"))


def _killpg(proc) -> None:
    try:
        os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
    except ProcessLookupError:
        pass


def _session_for(args) -> str | None:
    return getattr(args, "session", None) or os.environ.get("ORC_SESSION") or None


def cmd_rpc(args) -> int:
    if not quota_gate(args.force):
        return 3
    rd = registry.new_run(args.task, brain=args.brain, cwd=args.cwd,
                          session=_session_for(args))
    meta = registry.read_meta(rd)
    idle_timeout = args.idle_timeout
    if idle_timeout is None:
        idle_timeout = float(quota.load_config().get("idle_timeout_sec", 300))
    killed = False
    code = 0
    with (rd / "output.log").open("ab") as log:
        try:
            proc = subprocess.Popen(
                RPC_BASE, cwd=meta["cwd"], stdin=subprocess.PIPE,
                stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                start_new_session=True)
        except FileNotFoundError:
            log.write(b"orc: pi executable not found on PATH\n")
            print("orc: pi executable not found on PATH", file=sys.stderr)
            finalize(rd, meta, 127)
            return 127
        meta["pid"] = proc.pid
        meta["status"] = "running"
        registry.write_meta(rd, meta)
        proc.stdin.write(json.dumps(
            {"type": "prompt", "message": meta["task"]}).encode() + b"\n")
        proc.stdin.flush()

        sel = selectors.DefaultSelector()
        sel.register(proc.stdout, selectors.EVENT_READ)
        last_output = time.monotonic()
        buf = b""
        done = False
        usage = None
        try:
            while not done:
                if _inbox_has_kill(rd):
                    _killpg(proc)
                    killed = True
                    break
                events = sel.select(timeout=0.3)
                if not events:
                    if proc.poll() is not None:
                        break
                    if idle_timeout > 0 and time.monotonic() - last_output > idle_timeout:
                        msg = f"\norc: idle timeout after {int(idle_timeout)}s — killing worker\n"
                        log.write(msg.encode())
                        log.flush()
                        print(msg, file=sys.stderr)
                        _killpg(proc)
                        proc.wait()
                        finalize(rd, registry.read_meta(rd), 124)
                        return 124
                    continue
                chunk = os.read(proc.stdout.fileno(), 65536)
                if not chunk:
                    break
                last_output = time.monotonic()
                buf += chunk
                while b"\n" in buf:
                    line, buf = buf.split(b"\n", 1)
                    log.write(line + b"\n")
                    log.flush()
                    try:
                        evt = json.loads(line)
                    except ValueError:
                        sys.stdout.buffer.write(line + b"\n")
                        sys.stdout.buffer.flush()
                        continue
                    text = _extract_text(evt)
                    if text:
                        sys.stdout.write(text)
                        sys.stdout.flush()
                    if evt.get("type") in TERMINAL_EVENTS:
                        usage = _extract_usage(evt)
                        done = True
        except KeyboardInterrupt:
            _killpg(proc)
            killed = True
        finally:
            sel.close()
            try:
                proc.stdin.close()
            except (OSError, BrokenPipeError):
                pass
            if killed:
                _killpg(proc)
            code = proc.wait()
    print()
    finalize(rd, registry.read_meta(rd), -signal.SIGTERM if killed else code, usage)
    return 130 if killed else max(code, 0)


def _extract_usage(evt: dict):
    """Pull real token usage from agent_end's assistant messages (pi records it)."""
    best = None
    for msg in evt.get("messages", []) or []:
        u = msg.get("usage") if isinstance(msg, dict) else None
        if isinstance(u, dict) and u.get("totalTokens"):
            best = {"input": u.get("input", 0), "output": u.get("output", 0),
                    "cache_read": u.get("cacheRead", 0),
                    "total": u.get("totalTokens", 0)}
            cost = u.get("cost")
            if isinstance(cost, dict) and cost.get("total") is not None:
                best["cost_usd"] = round(cost["total"], 6)
    return best


def cmd_run(args) -> int:
    if not quota_gate(args.force):
        return 3
    rd = registry.new_run(args.task, brain=args.brain, cwd=args.cwd,
                          session=_session_for(args))
    if args.name:
        meta = registry.read_meta(rd)
        meta["name"] = args.name
        registry.write_meta(rd, meta)
    if args.bg:
        cmd = [sys.executable, "-m", "orc_pkg", "_exec", str(rd)]
        if args.idle_timeout is not None:
            cmd.extend(["--idle-timeout", str(args.idle_timeout)])
        subprocess.Popen(
            cmd,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        print(rd.name)
        return 0
    return _exec(rd, echo=True, idle_timeout=args.idle_timeout)


def cmd_exec(args) -> int:
    return _exec(Path(args.run_dir), echo=args.echo, idle_timeout=args.idle_timeout)
