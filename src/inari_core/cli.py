"""`inari-core`: inspect and handle capsules, export the schemas."""

from __future__ import annotations

import argparse
import sys
import tempfile
from pathlib import Path

from . import schema
from .capsule import io


def _open(path: Path, work: Path) -> Path:
    if path.is_dir():
        return path
    io.unpack(path, work)
    return work


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="inari-core")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("schema", help="write the JSON Schemas").add_argument(
        "dest", type=Path
    )
    for name in ("verify", "inspect"):
        sub.add_parser(
            name, help=f"{name} a capsule (directory or archive)"
        ).add_argument("path", type=Path)
    pack = sub.add_parser("pack", help="archive a capsule directory")
    pack.add_argument("root", type=Path)
    pack.add_argument("archive", type=Path)
    unpack = sub.add_parser("unpack", help="extract a capsule archive")
    unpack.add_argument("archive", type=Path)
    unpack.add_argument("dest", type=Path)
    args = parser.parse_args(argv)

    if args.cmd == "schema":
        for path in schema.export(args.dest):
            print(path)
        return 0
    if args.cmd == "pack":
        io.pack(args.root, args.archive)
        return 0
    if args.cmd == "unpack":
        io.unpack(args.archive, args.dest)
        return 0
    with tempfile.TemporaryDirectory() as tmp:
        root = _open(args.path, Path(tmp) / "capsule")
        if args.cmd == "verify":
            problems = io.verify(root)
            for problem in problems:
                print(problem, file=sys.stderr)
            print("ok" if not problems else f"{len(problems)} problem(s)")
            return 1 if problems else 0
        capsule = io.load(root)
        m = capsule.manifest
        holder = f"{m.holder.node_id} (epoch {m.holder.epoch})" if m.holder else "none"
        print(f"capsule  {m.capsule_id}\nthread   {m.thread_id}\nholder   {holder}")
        print(f"events   {len(capsule.timeline)}\nfiles    {len(m.files)}")
        print(f"snapshot {len(capsule.snapshot.tree)} workspace file(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
