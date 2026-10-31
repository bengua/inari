"""The session the agent opens: on Gemini Live, the model the core's settings name reaches the
plugin, and that name is one the plugin knows."""

import os
import unittest

from livekit.plugins.google.realtime import realtime_api

from kitsune_voice.agent import session_for


class SessionTest(unittest.TestCase):
    def test_gemini_live_gets_the_model_set_in_the_core(self):
        os.environ.setdefault("GOOGLE_API_KEY", "test")
        voice = {"mode": "gemini", "model": "gemini-3.1-flash-live-preview", "language": "fr-FR"}
        s = session_for(voice, {})
        self.assertEqual(s.llm.model, "gemini-3.1-flash-live-preview")
        self.assertIn(voice["model"], realtime_api.KNOWN_GEMINI_API_MODELS)


if __name__ == "__main__":
    unittest.main()
