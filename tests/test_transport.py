from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

from inari_core.capsule import verify
from inari_core.errors import TransportError
from inari_core.mesh import Bus
from inari_core.teleport import CapsuleHost, teleport
from inari_core.transport import BusClient, BusServer, EndpointServer, RemoteEndpoint
from tests.conftest import make_capsule

KEY = b"mesh-shared-key"


async def until(predicate, timeout=3.0):
    loop = asyncio.get_running_loop()
    end = loop.time() + timeout
    while loop.time() < end:
        if predicate():
            return True
        await asyncio.sleep(0.02)
    return predicate()


@pytest.fixture
async def remote(tmp_path: Path):
    src = CapsuleHost("node-a", tmp_path / "a")
    dst = CapsuleHost("node-b", tmp_path / "b")
    agent = src.adopt(make_capsule(tmp_path / "seed").root)
    server = EndpointServer(dst, KEY)
    port = await server.start()
    yield src, dst, agent, port
    await server.stop()


async def test_teleport_across_a_real_socket(remote):
    src, dst, agent, port = remote
    target = RemoteEndpoint("node-b", "127.0.0.1", port, KEY)
    out = await teleport(src, target, agent)
    assert out.state == "committed"
    assert verify(dst.agents / agent) == []
    assert await dst.holder(agent) == ("node-b", 2)


async def test_a_large_file_travels_in_chunks(tmp_path):
    src = CapsuleHost("node-a", tmp_path / "a")
    dst = CapsuleHost("node-b", tmp_path / "b")
    body = "0123456789abcdef" * 150_000  # 2.4 MB: several chunks
    agent = src.adopt(make_capsule(tmp_path / "seed", files={"big.txt": body}).root)
    server = EndpointServer(dst, KEY)
    port = await server.start()
    out = await teleport(src, RemoteEndpoint("node-b", "127.0.0.1", port, KEY), agent)
    await server.stop()
    assert out.state == "committed"
    sha = (dst.agents / agent / "artifacts/snapshot.json").read_text()
    assert "big.txt" in sha and verify(dst.agents / agent) == []


async def test_the_wrong_key_is_refused_and_the_source_resumes(remote):
    src, dst, agent, port = remote
    target = RemoteEndpoint("node-b", "127.0.0.1", port, b"another-key", timeout=2)
    out = await teleport(src, target, agent)
    assert out.state == "rolled_back"
    assert src.status(agent) == "running"
    assert not list(dst.agents.iterdir())


async def test_an_unreachable_target_rolls_back(remote):
    src, _, agent, _ = remote
    out = await teleport(
        src, RemoteEndpoint("node-b", "127.0.0.1", 1, KEY, timeout=1), agent
    )
    assert out.state == "rolled_back" and src.status(agent) == "running"


async def test_remote_application_errors_keep_their_type(remote):
    src, dst, agent, port = remote
    await teleport(src, RemoteEndpoint("node-b", "127.0.0.1", port, KEY), agent)
    other = CapsuleHost("node-c", src.root.parent / "c")
    other.adopt(src.agents / agent)  # still believes epoch 1
    out = await teleport(other, RemoteEndpoint("node-b", "127.0.0.1", port, KEY), agent)
    assert out.state == "rolled_back" and "StaleEpoch" in out.error


async def test_buses_talk_over_tcp_and_a_tampered_peer_is_dropped():
    home, phone = Bus("home"), Bus("phone")
    server = BusServer(home, KEY)
    port = await server.start()
    client = BusClient(phone, KEY, "127.0.0.1", port)
    client.start()
    got = []
    home.subscribe("teleport.request", lambda m: got.append(m.payload["n"]))
    assert await until(lambda: client.link.connected)
    await phone.publish("teleport.request", {"n": 1})
    assert await until(lambda: got == [1])
    intruder = BusClient(Bus("evil"), b"wrong", "127.0.0.1", port)
    intruder.start()
    await asyncio.sleep(0.4)
    assert not intruder.link.connected
    await intruder.stop()
    await client.stop()
    await server.stop()


async def test_durable_messages_wait_out_a_server_restart_and_arrive_once():
    home, phone = Bus("home"), Bus("phone")
    server = BusServer(home, KEY)
    port = await server.start()
    client = BusClient(phone, KEY, "127.0.0.1", port, retry=0.05)
    client.start()
    got = []
    home.subscribe("teleport.>", lambda m: got.append(m.payload["n"]))
    assert await until(lambda: client.link.connected)
    await server.stop()
    assert await until(lambda: not client.link.connected)
    await phone.publish("teleport.request", {"n": 1})  # durable: queued
    await phone.publish("load.x", {"n": 99})  # not durable: lost
    assert got == []
    server2 = BusServer(home, KEY, port=port)
    await server2.start()
    assert await until(lambda: got == [1], timeout=5)
    await asyncio.sleep(0.3)
    assert got == [1]
    await client.stop()
    await server2.stop()


def test_transport_error_is_importable():
    assert issubclass(TransportError, Exception)
