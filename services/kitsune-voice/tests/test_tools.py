"""The voice agent's tools against a stand-in core: only the allowed ones are offered, and each
calls the core's Kitsune API as the page does."""

import asyncio
import json
import unittest

import httpx

from kitsune_voice import tools
from kitsune_voice.core import Core


class FakeCore:
    def __init__(self):
        self.calls = []

    def handler(self, request: httpx.Request) -> httpx.Response:
        assert request.url.params["token"] == "owner"
        body = json.loads(request.content or b"{}")
        self.calls.append(body)
        action = body.get("action")
        if action == "items":
            return httpx.Response(200, json={"items": [{"id": 4, "text": "Café"}, {"id": 5, "text": "Lait"}]})
        if action == "add_items":
            return httpx.Response(200, json={"ids": [1, 2]})
        if action == "capture_text":
            return httpx.Response(200, json={"id": 9})
        if action == "assist":
            return httpx.Response(400, json={"error": "no core of the mesh runs the local engine"})
        return httpx.Response(200, json={"changed": 1, "reminders": []})


def run(c):
    return asyncio.run(c)


class ToolsTest(unittest.TestCase):
    def setUp(self):
        self.fake = FakeCore()
        client = httpx.AsyncClient(transport=httpx.MockTransport(self.fake.handler))
        self.core = Core("http://core", "owner", client)

    def test_only_the_allowed_tools_are_offered(self):
        names = {t.info.name if hasattr(t, "info") else t.__name__ for t in tools.build(self.core, ["note", "items"])}
        self.assertEqual(names, {"keep_note", "read_list"})

    def test_each_tool_calls_the_brain(self):
        every = {getattr(t, "__name__", ""): t for t in tools.build(self.core, list(tools.NEEDS.values()))}
        self.assertIn("kept (note 9)", run(every["keep_note"](None, "racheter du café")))
        self.assertEqual(run(every["add_to_list"](None, "Courses", "shopping", ["café", "lait"])), "2 added to Courses")
        self.assertEqual(run(every["check_items"](None, "Courses", ["café"])), "1 done")
        self.assertEqual(self.fake.calls[-1], {"action": "check", "ids": [4], "done": True})
        self.assertIn("not started", run(every["start_assistant"](None, "daily-plan")))
        self.assertIn("unknown assistant", run(every["start_assistant"](None, "rm -rf")))


if __name__ == "__main__":
    unittest.main()
