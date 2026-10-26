from __future__ import annotations

import asyncio
import base64
import json
from pathlib import Path

from inari_core.agent import AcpHarness, AgentRunner, Completion
from inari_core.capsule import load, verify
from inari_core.gateway import Gateway
from inari_core.mesh import Bus
from inari_core.sandbox import SubprocessRuntime
from inari_core.secrets import EnvResolver
from inari_core.teleport import CapsuleHost
from inari_core.usage import CloudroomUsageSource, StaticUsageSource

from .test_acp import FAKE, prompts
from .test_agent import Scripted
from .test_gateway import http


async def fake_cloudroom():
    async def handle(reader, writer):
        head = (await reader.readuntil(b"\r\n\r\n")).decode().split("\r\n")
        line = head[0]
        headers = {k.lower(): v for k, v in (h.split(": ", 1) for h in head[1:] if ": " in h)}
        size = int(headers.get("content-length", 0))
        body = json.loads(await reader.readexactly(size)) if size else {}
        if "/hosts" in line:
            reply = [{"id": "h1", "status": "connected"}]
        elif "getUsage" in line:
            reply = {
                "result": {
                    "machines": [
                        {
                            "providers": [
                                {
                                    "providerId": body["providerId"],
                                    "displayName": "Claude Code",
                                    "logoUrl": "/logo.svg",
                                    "strings": {"iconTint": {"dark": "#D97757"}},
                                    "usage": {
                                        "status": "ok",
                                        "planLabel": "Pro",
                                        "windows": [
                                            {
                                                "label": "Five-hour limit",
                                                "usedPercent": 41,
                                                "resetsAt": "2026-01-01T00:00:00Z",
                                            }
                                        ],
                                    },
                                }
                            ]
                        }
                    ]
                }
            }
        else:
            reply = None
        raw = b"<svg/>" if "logo.svg" in line else json.dumps(reply).encode()
        writer.write(
            b"HTTP/1.1 200 OK\r\nContent-Length: %d\r\nConnection: close\r\n\r\n" % len(raw) + raw
        )
        await writer.drain()
        writer.close()

    server = await asyncio.start_server(handle, "127.0.0.1", 0)
    return server, server.sockets[0].getsockname()[1]


async def test_usage_sources_normalise_quotas_and_a_logo_is_proxied():
    server, port = await fake_cloudroom()
    source = CloudroomUsageSource(f"http://127.0.0.1:{port}", ["claude-code"])
    items = await source.fetch()
    assert (
        items[0].provider_id == "claude" and items[0].plan == "Pro" and items[0].tint == "#D97757"
    )
    assert items[0].windows[0].used_percent == 41
    gateway = Gateway(
        Bus("hub"),
        token="t",
        device_key=b"k",
        usage_sources=[
            source,
            StaticUsageSource([{"provider_id": "local", "name": "Local models"}]),
        ],
        usage_interval=0.05,
    )
    await gateway.start()
    for _ in range(60):
        if len(gateway.usage) == 2:
            break
        await asyncio.sleep(0.05)
    snap = gateway.snapshot()
    assert {u["provider_id"] for u in snap["usage"]} == {"claude", "local"}
    status, body = await http(gateway.port, "GET", "/api/logo/claude", token="t")
    assert status == 200 and body == b"<svg/>"
    await gateway.stop()
    server.close()


async def test_the_mascot_folder_is_served_but_never_outside_itself(tmp_path: Path):
    (tmp_path / "fox").mkdir()
    (tmp_path / "fox" / "index.html").write_text("<html>fox</html>")
    (tmp_path / "secret.txt").write_text("nope")
    gateway = Gateway(Bus("hub"), token="t", device_key=b"k", mascot_dir=tmp_path / "fox")
    await gateway.start()
    status, body = await http(gateway.port, "GET", "/fox/index.html", token="")
    assert status == 200 and b"fox" in body
    assert (await http(gateway.port, "GET", "/fox/../secret.txt", token=""))[0] in (401, 404)
    assert gateway.snapshot()["ui"]["mascot"] is True
    plain = Gateway(Bus("hub2"), token="t", device_key=b"k")
    assert plain.snapshot()["ui"]["mascot"] is False
    await gateway.stop()


async def test_choosing_a_model_is_remembered_journaled_and_sent_to_the_agent(tmp_path):
    host = CapsuleHost("node-a", tmp_path / "a")
    log = tmp_path / "log"
    harness = AcpHarness(
        [__import__("sys").executable, FAKE],
        host.root / "acp.json",
        extra_env={"FAKE_ACP_LOG": str(log)},
    )
    run = AgentRunner(host, None, SubprocessRuntime(EnvResolver()), harness=harness)
    agent = run.create("t1")
    assert run.set_model(agent, "fam/beta") is True and run.set_model(agent, "fam/beta") is False
    assert load(host.agents / agent).manifest.labels["model"] == "fam/beta"
    await run.run(agent, "hello")
    assert {"set_model": "fam/beta"} in prompts(log)
    kinds = [e.kind.value for e in load(host.agents / agent).timeline.ordered()]
    assert "provider_switch" in kinds and verify(host.agents / agent) == []
    assert await harness.list_models(tmp_path) == ["fam/alpha", "fam/beta"]


async def test_the_model_loop_uses_the_thread_model(tmp_path):
    seen = []

    class Spy(Scripted):
        async def complete(self, rendered, tools):
            seen.append(rendered.get("model"))
            return await super().complete(rendered, tools)

    host = CapsuleHost("node-a", tmp_path / "a")
    run = AgentRunner(host, Spy(Completion("ok")), SubprocessRuntime(EnvResolver()))
    agent = run.create("t1")
    run.set_model(agent, "gpt-x")
    await run.run(agent, "hi")
    assert seen == ["gpt-x"]


async def test_an_attached_file_lands_in_the_workspace_and_cannot_escape_it(tmp_path):
    host = CapsuleHost("node-a", tmp_path / "a")
    run = AgentRunner(host, Scripted(Completion("ok")), SubprocessRuntime(EnvResolver()))
    agent = run.create("t1")
    out = await run.upload(agent, "docs/notes.txt", b"attached text")
    assert out["ok"] and "docs/notes.txt" in load(host.agents / agent).snapshot.tree
    assert (await run.upload(agent, "../evil.txt", b"x"))["ok"] is False
    assert verify(host.agents / agent) == []
    _ = base64
