"""Kitsune's live voice: a LiveKit voice agent that joins the owner's room and talks with them.

Two modes, chosen in the GUI (Kitsune settings, voice.mode):
- gemini: Gemini Live, a realtime speech-to-speech model in Google's cloud. Low latency and no
  load on the person's GPU, but their voice and what the agent reads go to Google (on the free
  tier, Google may use it to improve its products).
- private: everything on the person's own servers: their speech-to-text (Whisper), their small
  language model, their voice (Kokoro or any OpenAI-compatible /audio/speech server).
"""

from __future__ import annotations

import asyncio
import logging
import os

from livekit.agents import Agent, AgentSession, JobContext, WorkerOptions, cli
from livekit.plugins import google, openai, silero

from . import tools
from .core import Core

log = logging.getLogger("kitsune-voice")

INSTRUCTIONS = """Tu es Kitsune, l'assistant vocal d'une personne, souvent TDAH. Tu parles français,
court et chaleureux, comme un majordome bienveillant. Aujourd'hui : {now}.

Comment tu aides (d'après docs/tdah-recherche-fr.md) :
- Rien ne doit rester dans sa tête : ce qu'elle dit d'important, tu le gardes (keep_note).
- Préfère un rappel lié à un lieu ou un moment (« quand tu arrives au bureau ») à une heure nue.
- Pour s'organiser : au plus trois priorités, chacune avec son plus petit premier pas, sa durée,
  formulée « quand…, alors… ». Pour une vraie journée à planifier, lance l'assistant daily-plan.
- Rends le temps visible : « dans 20 minutes », pas seulement « à 9 h ».
- Jamais de culpabilité : rien n'est « en retard », tout se « replace ».
- Tu ne donnes aucun conseil médical.
Les outils parlent au cerveau de la personne ; dis simplement ce que tu as fait."""


def session_for(voice: dict, settings: dict) -> AgentSession:
    if voice.get("mode") == "gemini":
        kwargs = {"voice": voice.get("voice") or "Puck", "language": voice.get("language") or "fr-FR"}
        if voice.get("model"):
            kwargs["model"] = voice["model"]
        # the key comes from the environment (GOOGLE_API_KEY), never from the core's settings
        return AgentSession(llm=google.realtime.RealtimeModel(**kwargs))
    asr, slm, tts = settings.get("asr", {}), settings.get("slm", {}), settings.get("tts", {})
    if not (asr.get("url") and slm.get("url") and tts.get("url")):
        raise RuntimeError("private mode needs Kitsune's speech-to-text, language model and voice servers set in the GUI")
    local = "local"  # the local servers take any key
    return AgentSession(
        vad=silero.VAD.load(),
        stt=openai.STT(base_url=asr["url"], api_key=local, model=asr.get("model") or "whisper-1",
                       language=(asr.get("language") or "fr")[:2]),
        llm=openai.LLM(base_url=slm["url"], api_key=local, model=slm.get("model") or "local"),
        tts=openai.TTS(base_url=tts["url"], api_key=local, model=tts.get("model") or "kokoro",
                       voice=tts.get("voice") or "ff_siwis"),
    )


async def entrypoint(ctx: JobContext) -> None:
    core = Core.from_env()
    status = await core.status()
    settings = status.get("settings", {})
    voice = settings.get("voice", {})
    allowed = list(voice.get("tools") or [])
    await ctx.connect()
    session = session_for(voice, settings)
    agent = Agent(instructions=INSTRUCTIONS.format(now=tools.now_line()), tools=tools.build(core, allowed))
    await session.start(agent=agent, room=ctx.room)
    await session.generate_reply(instructions="Salue la personne en une phrase courte et demande ce qu'elle veut garder ou faire.")


def main() -> None:
    logging.basicConfig(level=logging.INFO)
    # the LiveKit server and its key come from the core's settings, so they are set in one place
    voice = asyncio.run(Core.from_env().status()).get("settings", {}).get("voice", {})
    for env, key in (("LIVEKIT_URL", "url"), ("LIVEKIT_API_KEY", "api_key"), ("LIVEKIT_API_SECRET", "api_secret")):
        if not os.environ.get(env) and voice.get(key):
            os.environ[env] = voice[key]
    cli.run_app(WorkerOptions(entrypoint_fnc=entrypoint))


if __name__ == "__main__":
    main()
