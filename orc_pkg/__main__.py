import argparse
import sys

from orc_pkg import VERSION


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="orc", description="pi-orchestra: MiniMax M3 worker delegation")
    sub = p.add_subparsers(dest="cmd")
    sub.add_parser("version", help="print version")
    return p


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)
    if args.cmd == "version":
        print(f"orc {VERSION}")
        return 0
    build_parser().print_help()
    return 1


if __name__ == "__main__":
    sys.exit(main())
