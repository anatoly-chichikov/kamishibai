"""Runtime wiring helpers for kamishibai."""

import json
import os
from importlib.resources import files

from google import genai

from .config import profiles
from .media import Audio
from .media import Illustration
from .media import TtsVoice
from .scene import BorderDetector
from .scene import Cache
from .scene import MangaRenderer
from .scene import SceneTranslator
from .scene import TextDetector
from .scene import TextDetectors


def assets():
    """Return the traversable resource container for packaged assets."""
    return files("kamishibai.assets")


def text(name):
    """Load a packaged text asset by filename."""
    return (assets() / name).read_text(encoding="utf-8").strip()


def audio_prompt(language):
    """Return the shared audio prompt template for a target language."""
    return text("audio_prompt.txt").replace("{language}", language)


def scene_prompt(language):
    """Return the shared scene prompt template for a target language."""
    return text("scene_prompt.txt").replace("{language}", language)


def template():
    """Load the packaged manga template JSON document."""
    return json.loads((assets() / "manga_template.json").read_text(encoding="utf-8"))


def client():
    """Build a Gemini API client from GEMINI_API_KEY."""
    key = os.environ.get("GEMINI_API_KEY")
    if not key:
        raise ValueError("GEMINI_API_KEY environment variable is not set; export it before running")
    return genai.Client(api_key=key)


class Media:
    """Builds per-target audio and illustration services lazily."""

    def __init__(self, client, cache, items=None):
        self._client = client
        self._cache = str(cache)
        self._items = items if items is not None else profiles()
        self._detectors = {}
        self._renderer = MangaRenderer(
            client,
            retries=3,
            text=TextDetectors(
                self._detectors,
                TextDetector(60, self._items.fallback_ocr()),
            ),
            border=BorderDetector(width=6, brightness=240, margin=10),
        )
        self._audio = {}
        self._illustration = {}
        self._translator = {}

    def audio(self, entry):
        """Return the audio service for the entry target language."""
        code = entry["target_lang"]
        if code not in self._audio:
            item = self._items.item(code)
            self._audio[code] = Audio(
                self._client,
                Cache(item.audio().cache(), self._cache),
                audio_prompt(item.audio().language()),
                TtsVoice(("gemini-2.5-flash-preview-tts", "gemini-2.5-pro-preview-tts")),
            )
        return self._audio[code]

    def illustration(self, entry):
        """Return the illustration service for the entry target language."""
        code = entry["target_lang"]
        if code not in self._illustration:
            item = self._items.item(code)
            if code not in self._translator:
                self._translator[code] = SceneTranslator(
                    self._client,
                    scene_prompt(item.audio().language()),
                    template(),
                )
            if code not in self._detectors:
                self._detectors[code] = TextDetector(60, item.imagery().ocr())
            self._illustration[code] = Illustration(
                Cache(item.imagery().cache(), self._cache),
                self._translator[code],
                self._renderer,
            )
        return self._illustration[code]
