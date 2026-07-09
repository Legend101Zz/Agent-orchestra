import argparse
import sys

from orc_pkg import VERSION


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="orc", description="pi-orchestra: MiniMax M3 worker delegation")
    sub = p.add_subparsers(dest="cmd")
    sub.add_parser("version", help="print version")

    run = sub.add_parser("run", help="delegate a one-shot task to pi/MiniMax-M3")
    run.add_argument("task")
    run.add_argument("--cwd", default=None)
    run.add_argument("--brain", default="human", choices=["claude", "codex", "human"])
    run.add_argument("--name", default=None)
    run.add_argument("--bg", action="store_true")
    run.add_argument("--force", action="store_true")
    run.add_argument("--idle-timeout", type=float, default=None, dest="idle_timeout",
                     help="kill worker after N seconds without output (default: config idle_timeout_sec)")

    ex = sub.add_parser("_exec")  # hidden: executes a registered run dir
    ex.add_argument("run_dir")
    ex.add_argument("--echo", action="store_true")
    ex.add_argument("--idle-timeout", type=float, default=None, dest="idle_timeout")

    return p


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    if args.cmd == "version":
        print(f"orc {VERSION}")
        return 0
    if args.cmd == "run":
        from orc_pkg import runner
        return runner.cmd_run(args)
    if args.cmd == "_exec":
        from orc_pkg import runner
        return runner.cmd_exec(args)
    build_parser().print_help()
    return 1


if __name__ == "__main__":
    sys.exit(main())
