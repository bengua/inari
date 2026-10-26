"""The tools an agent may call. File tools are confined to the workspace; `bash`
goes through the sandbox runtime."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from ..capsule.models import EnvironmentSpec
from ..sandbox import Runtime
from .providers import ToolSpec

MAX_OUTPUT = 20_000

SPECS = [
    ToolSpec(
        "bash",
        "Run a shell command in the workspace.",
        {"type": "object", "properties": {"cmd": {"type": "string"}}, "required": ["cmd"]},
    ),
    ToolSpec(
        "read_file",
        "Read a text file from the workspace.",
        {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]},
    ),
    ToolSpec(
        "write_file",
        "Create or replace a text file in the workspace.",
        {
            "type": "object",
            "properties": {"path": {"type": "string"}, "content": {"type": "string"}},
            "required": ["path", "content"],
        },
    ),
    ToolSpec(
        "list_dir",
        "List a directory of the workspace.",
        {"type": "object", "properties": {"path": {"type": "string"}}},
    ),
]


def command_text(name: str, args: dict[str, Any]) -> str:
    """What an approval pattern is matched against."""
    return f"{name} {args.get('cmd') or args.get('path') or ''}".strip()


class Toolbox:
    def __init__(self, workdir: Path, spec: EnvironmentSpec, runtime: Runtime) -> None:
        self.workdir, self.spec, self.runtime = Path(workdir).resolve(), spec, runtime

    def _inside(self, rel: str) -> Path:
        target = (self.workdir / rel).resolve()
        if target != self.workdir and self.workdir not in target.parents:
            raise ValueError(f"path leaves the workspace: {rel}")
        return target

    async def run(self, name: str, args: dict[str, Any]) -> tuple[str, bool]:
        """(output, is_error). Never raises: a failing tool is information for the model."""
        try:
            if name == "bash":
                result = await self.runtime.run(
                    self.spec, self.workdir, ["/bin/sh", "-c", str(args["cmd"])], timeout=60
                )
                out = (result.stdout + result.stderr)[:MAX_OUTPUT]
                if result.timed_out:
                    return out + "\n[timed out]", True
                return out or "(no output)", result.code != 0
            if name == "read_file":
                return self._inside(str(args["path"])).read_text()[:MAX_OUTPUT], False
            if name == "write_file":
                path = self._inside(str(args["path"]))
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(str(args["content"]))
                return f"wrote {len(str(args['content']))} bytes", False
            if name == "list_dir":
                base = self._inside(str(args.get("path", ".")))
                return "\n".join(
                    sorted(p.name + ("/" if p.is_dir() else "") for p in base.iterdir())
                ), False
            return f"unknown tool {name}", True
        except (OSError, ValueError, KeyError) as exc:
            return f"{type(exc).__name__}: {exc}", True
