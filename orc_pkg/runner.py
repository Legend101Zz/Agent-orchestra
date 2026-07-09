"""Spawn pi workers: foreground/background runs with quota gating and idle watchdog."""
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


def finalize(rd: Path, meta: dict, code: int) -> None:
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
    registry.write_meta(rd, meta)


def _exec(rd: Path, echo: bool = False, idle_timeout: float | None = None) -> int:
    meta = registry.read_meta(rd)
    if idle_timeout is None:
        idle_timeout = float(quota.load_config().get("idle_timeout_sec", 300))

    log_file = open(rd / "output.log", "ab")
    code = 0
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
                            finalize(rd, meta, 124)
                            return 124
                        continue
                    chunk = os.read(proc.stdout.fileno(), 65536)
                    if not chunk:
                        break
                    log_file.write(chunk)
                    log_file.flush()
                    if echo:
                        sys.stdout.buffer.write(chunk)
                        sys.stdout.buffer.flush()
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

    finalize(rd, meta, code)
    if code >= 0:
        return max(code, 0)
    return 130


def cmd_run(args) -> int:
    if not quota_gate(args.force):
        return 3
    rd = registry.new_run(args.task, brain=args.brain, cwd=args.cwd)
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
