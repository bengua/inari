"""The voice agent end to end, in text: a real Inari core (INARI_E2E_URL, with INARI_E2E_TOKEN),
LiveKit Agents' own session, and a stand-in OpenAI-compatible model that asks for a tool. The
shopping list the agent fills is read back from the core. Skipped when no core is given (CI runs
the tool tests; a core runs this one: see README)."""

import asyncio
import json
import os
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

URL = os.environ.get("INARI_E2E_URL", "")
TOKEN = os.environ.get("INARI_E2E_TOKEN", "")


class Model(BaseHTTPRequestHandler):
    """First request: call add_to_list; then: a short answer."""

    calls = 0

    def log_message(self, *a):
        pass

    def do_POST(self):
        n = int(self.headers.get("content-length", 0))
        self.rfile.read(n)
        Model.calls += 1
        if Model.calls == 1:
            delta = {"tool_calls": [{"index": 0, "id": "call_1", "type": "function", "function": {
                "name": "add_to_list",
                "arguments": json.dumps({"list_name": "Courses", "kind": "shopping", "items": ["café"]})}}]}
            finish = "tool_calls"
        else:
            delta, finish = {"content": "C'est sur la liste."}, "stop"
        events = [
            {"id": "x", "object": "chat.completion.chunk", "created": 0, "model": "m",
             "choices": [{"index": 0, "delta": {"role": "assistant", **delta}, "finish_reason": None}]},
            {"id": "x", "object": "chat.completion.chunk", "created": 0, "model": "m",
             "choices": [{"index": 0, "delta": {}, "finish_reason": finish}]},
        ]
        body = "".join(f"data: {json.dumps(e)}\n\n" for e in events) + "data: [DONE]\n\n"
        raw = body.encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)


@unittest.skipUnless(URL and TOKEN, "set INARI_E2E_URL and INARI_E2E_TOKEN to run against a core")
class VoiceEndToEnd(unittest.TestCase):
    def test_the_voice_fills_the_shopping_list_in_the_core(self):
        from livekit.agents import Agent, AgentSession
        from livekit.plugins import openai

        from kitsune_voice import tools
        from kitsune_voice.agent import INSTRUCTIONS
        from kitsune_voice.core import Core

        server = ThreadingHTTPServer(("127.0.0.1", 0), Model)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        llm_url = f"http://127.0.0.1:{server.server_address[1]}/v1"

        async def run():
            core = Core(URL, TOKEN)
            agent = Agent(instructions=INSTRUCTIONS.format(now=tools.now_line()),
                          tools=tools.build(core, ["note", "reminders", "lists", "items", "add_items", "check"]))
            async with openai.LLM(base_url=llm_url, api_key="local", model="m") as llm, AgentSession(llm=llm) as session:
                await session.start(agent)
                result = await session.run(user_input="Ajoute du café à la liste de courses")
                result.expect.next_event().is_function_call(name="add_to_list")
                result.expect.next_event().is_function_call_output()
            return await core.call("items", list="Courses")

        items = asyncio.run(run())
        server.shutdown()
        self.assertIn("café", [i["text"] for i in items["items"]])


if __name__ == "__main__":
    unittest.main()
