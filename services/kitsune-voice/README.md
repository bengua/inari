# kitsune-voice

Kitsune's live voice: a [LiveKit](https://livekit.io) voice agent (LiveKit Agents, Apache-2.0)
that joins the owner's room when the Kitsune page starts a live talk, and answers there.

- **gemini**: Gemini Live, realtime speech-to-speech in Google's cloud. Low latency, no load on
  your GPU; your voice and what the agent reads go to Google (on the free tier, Google may use them
  to improve its products). The key is `GOOGLE_API_KEY` in this service's environment.
- **private**: your own servers only: Kitsune's speech-to-text (Whisper), its small language model,
  its voice (Kokoro or any OpenAI-compatible `/audio/speech` server).

The mode, the Gemini model and voice, the LiveKit server and the tools the voice may use with your
brain are Kitsune settings, set on the Kitsune page (Local servers → Live voice). This service only
needs to reach the core:

```text
INARI_URL=http://127.0.0.1:8765      # the core's gateway
INARI_TOKEN_FILE=~/.config/inari/token   # the owner's token (or INARI_TOKEN)
GOOGLE_API_KEY=…                      # gemini mode only
```

Run your own LiveKit server (Apache-2.0; `livekit-server --dev` for a first try, keys `devkey` /
`secret`), then:

```text
pip install .
kitsune-voice dev      # or: kitsune-voice start
```

Tests: `python -m unittest discover -s tests`. Against a running core, the end-to-end test drives
LiveKit Agents' own session in text with a stand-in model and checks that the shopping list in the
core is filled: `INARI_E2E_URL=http://127.0.0.1:8765 INARI_E2E_TOKEN=… python -m unittest tests.test_e2e`.
