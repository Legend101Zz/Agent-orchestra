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

    rp = sub.add_parser("rpc", help="streaming delegation via pi rpc mode")
    rp.add_argument("task")
    rp.add_argument("--cwd", default=None)
    rp.add_argument("--brain", default="human", choices=["claude", "codex", "human"])
    rp.add_argument("--force", action="store_true")
    rp.add_argument("--idle-timeout", type=float, default=None, dest="idle_timeout")

    ls = sub.add_parser("list", help="list delegated runs")
    ls.add_argument("--json", action="store_true")

    sh = sub.add_parser("show", help="show a run's meta and log tail")
    sh.add_argument("id")
    sh.add_argument("--tail", type=int, default=40)

    kl = sub.add_parser("kill", help="kill a running delegation")
    kl.add_argument("id")

    qt = sub.add_parser("quota", help="MiniMax coding-plan quota")
    qt.add_argument("--json", action="store_true")
    qt.add_argument("--force", action="store_true", help="bypass 60s cache")

    return p


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    if args.cmd == "version":
        print(f"orc {VERSION}")
        return 0
    if args.cmd == "run":
        from orc_pkg import runner
        return runner.cmd_run(args)
    if args.cmd == "rpc":
        from orc_pkg import runner
        return runner.cmd_rpc(args)
    if args.cmd == "_exec":
        from orc_pkg import runner
        return runner.cmd_exec(args)
    if args.cmd in ("list", "show", "kill", "quota"):
        from orc_pkg import control
        return {"list": control.cmd_list, "show": control.cmd_show,
                "kill": control.cmd_kill, "quota": control.cmd_quota}[args.cmd](args)
    build_parser().print_help()
    return 1


if __name__ == "__main__":
    sys.exit(main())
