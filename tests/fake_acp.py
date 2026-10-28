"""A tiny ACP agent for tests: scripted by the words in the prompt."""

import json
import os
import sys
import time
from pathlib import Path

LOG = Path(os.environ["FAKE_ACP_LOG"])
KNOWN = LOG.with_suffix(".sessions")


def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def update(sid, body):
    send(
        {"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": sid, "update": body}}
    )


def known():
    return KNOWN.read_text().split() if KNOWN.exists() else []


for line in sys.stdin:
    msg = json.loads(line)
    if "method" not in msg:  # a reply to our permission request
        PERM_REPLY = msg
        continue
    m, ident, p = msg["method"], msg.get("id"), msg.get("params") or {}
    if m == "initialize":
        send(
            {
                "jsonrpc": "2.0",
                "id": ident,
                "result": {
                    "protocolVersion": 1,
                    "agentCapabilities": {
                        "promptCapabilities": {"image": bool(os.environ.get("FAKE_ACP_IMAGES"))}
                    },
                },
            }
        )
    elif m == "session/new":
        sid = f"s{len(known()) + 1}"
        KNOWN.write_text(" ".join([*known(), sid]))
        send(
            {
                "jsonrpc": "2.0",
                "id": ident,
                "result": {
                    "sessionId": sid,
                    "models": {
                        "availableModels": [{"modelId": "fam/alpha"}, {"modelId": "fam/beta"}],
                        "currentModelId": "fam/alpha",
                    },
                    "modes": {
                        "currentModeId": "default",
                        "availableModes": [
                            {"id": "default", "name": "Default", "description": "Asks first"},
                            {"id": "plan", "name": "Plan", "description": "Read-only"},
                        ],
                    },
                },
            }
        )
        update(
            sid,
            {
                "sessionUpdate": "available_commands_update",
                "availableCommands": [
                    {"name": "summarize", "description": "Summarize the thread"},
                    {"name": "review", "description": "Review the changes"},
                ],
            },
        )
    elif m == "session/set_mode":
        with LOG.open("a") as fh:
            fh.write(json.dumps({"set_mode": p["modeId"]}) + "\n")
        # like Gemini: the change is also announced as message text
        notice = {"type": "text", "text": "[MODE_UPDATE] " + p["modeId"]}
        update(p["sessionId"], {"sessionUpdate": "agent_message_chunk", "content": notice})
        send({"jsonrpc": "2.0", "id": ident, "result": {}})
    elif m == "session/load":
        if p["sessionId"] in known():
            send({"jsonrpc": "2.0", "id": ident, "result": {}})
        else:
            send(
                {
                    "jsonrpc": "2.0",
                    "id": ident,
                    "error": {"code": -32000, "message": "unknown session"},
                }
            )
    elif m == "session/prompt":
        sid = p["sessionId"]
        text = p["prompt"][0]["text"]
        with LOG.open("a") as fh:
            blocks = [b["type"] for b in p["prompt"]]
            fh.write(json.dumps({"cwd": os.getcwd(), "prompt": text, "blocks": blocks}) + "\n")
        last = text.rpartition("New message from the user:")[2]  # priming quotes earlier words
        if "orphan" in last:  # a child of its own, as an agent that runs a CLI
            import subprocess

            child = subprocess.Popen(["sleep", "300"])
            LOG.with_suffix(".child").write_text(str(child.pid))
        if "crash" in last:
            sys.exit(3)
        if "slow" in last:
            for _ in range(100):
                time.sleep(0.1)
        update(
            sid,
            {
                "sessionUpdate": "agent_thought_chunk",
                "content": {"type": "text", "text": "secret thoughts"},
            },
        )
        if "make" in last:
            Path("made.txt").write_text("made by the fake agent")
            update(
                sid,
                {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "k1",
                    "title": "write made.txt",
                    "kind": "edit",
                    "rawInput": {"path": "made.txt"},
                },
            )
            update(
                sid,
                {
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "k1",
                    "status": "completed",
                    "content": [
                        {"type": "content", "content": {"type": "text", "text": "ok written"}}
                    ],
                },
            )
        if "perm" in last:
            # "permstr": the request id is a string, as OpenFox's adapter sends it
            req_id = "ofx-1" if "permstr" in last else 900
            send(
                {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "method": "session/request_permission",
                    "params": {
                        "sessionId": sid,
                        "toolCall": {
                            "toolCallId": "k2",
                            "title": "bash: rm -rf x",
                            "kind": "execute",
                            "status": "pending",
                        },
                        "options": [
                            {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                            {"optionId": "deny", "name": "Deny", "kind": "reject_once"},
                        ],
                    },
                }
            )
            reply = json.loads(sys.stdin.readline())
            chosen = reply["result"]["outcome"].get("optionId")
            update(
                sid,
                {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": f"permission result: {chosen}"},
                },
            )
        else:
            # "long": a reply far longer than any summary, ending on a known word
            if "long" in last:
                tail = ("word " * 300).strip()
            elif "whereami" in last:
                tail = os.environ.get("HOME", "")
            else:
                tail = text.splitlines()[-1][:60]
            update(
                sid,
                {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "fake reply to: "},
                },
            )
            update(
                sid,
                {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": tail},
                },
            )
        cost = {"amount": 0.02, "currency": "USD"}
        update(sid, {"sessionUpdate": "usage_update", "used": 1200, "size": 8000, "cost": cost})
        send({"jsonrpc": "2.0", "id": ident, "result": {"stopReason": "end_turn"}})
    elif m == "session/set_model":
        with LOG.open("a") as fh:
            fh.write(json.dumps({"set_model": p["modelId"]}) + "\n")
        send({"jsonrpc": "2.0", "id": ident, "result": {}})
    elif m == "session/cancel":
        pass
