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
    sub.add_parser("keygen", help="make a signing key pair").add_argument("name")
    sign = sub.add_parser("sign", help="sign an exported capsule directory")
    sign.add_argument("root", type=Path)
    sign.add_argument("--key", type=Path, required=True)
    sub.choices["verify"].add_argument(
        "--trusted", type=Path, action="append", default=[],
        help="public key file; the capsule must be signed by one of them",
    )  # fmt: skip
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
    if args.cmd == "keygen":
        from . import signature

        private, public = signature.generate_keypair()
        key_file = Path(args.name + ".key")
        key_file.write_text(private)
        key_file.chmod(0o600)
        Path(args.name + ".pub").write_text(public)
        print(f"key id {signature.key_id(public)}")
        return 0
    if args.cmd == "sign":
        from . import signature

        print(signature.sign_capsule(args.root, args.key.read_text().strip()))
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
            if args.trusted:
                from . import signature

                trusted = {p.read_text().strip() for p in args.trusted}
                problems += signature.verify_signature(root, trusted)
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
