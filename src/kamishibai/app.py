#!/usr/bin/env python3
"""Unified command line application for deck generation."""

import argparse
import json
import os
import re
import sys
from datetime import datetime
from importlib.resources import files
from pathlib import Path

import genanki
from google import genai

from .deck import Audio
from .deck import CardModel
from .deck import FontFamily
from .deck import Illustration
from .deck import Pipeline
from .deck import Report
from .deck import StableId
from .deck import Thumbnail
from .deck import TtsVoice
from .deck import Vocabulary
from .deck import VocabularyDeck
from .deck import VocabularyLayout
from .deck import VocabularyMapping
from .deck import VocabularyNote
from .diagnosis import DiagnosisSelector
from .target import AudioProfile
from .target import DeckNaming
from .target import ImageProfile
from .target import TargetProfile
from .manga import BorderDetector
from .manga import Cache
from .manga import MangaRenderer
from .manga import SceneTranslator
from .manga import TextDetector
from .manga import TextDetectors
from .progress import ProgressSelector


def _profile(code):
    """Build the configured TargetProfile for a supported code."""
    profiles = {
        "de": TargetProfile(
            "de",
            AudioProfile("Say in natural German: {text}", "audio-de"),
            ImageProfile("eng+deu", "manga-de"),
            DeckNaming("German Vocabulary", "de", "kamishibai.json"),
        ),
        "el": TargetProfile(
            "el",
            AudioProfile("Say in natural Greek: {text}", "audio-el"),
            ImageProfile("eng+ell", "manga-el"),
            DeckNaming("Greek Vocabulary", "el", "kamishibai.json"),
        ),
        "en": TargetProfile(
            "en",
            AudioProfile("Say in natural English: {text}", "audio-en"),
            ImageProfile("eng", "manga-en"),
            DeckNaming("English Vocabulary", "en", "kamishibai.json"),
        ),
        "es": TargetProfile(
            "es",
            AudioProfile("Say in natural Spanish: {text}", "audio-es"),
            ImageProfile("eng+spa", "manga-es"),
            DeckNaming("Spanish Vocabulary", "es", "kamishibai.json"),
        ),
        "zh": TargetProfile(
            "zh",
            AudioProfile("Say in natural Mandarin Chinese: {text}", "audio-zh"),
            ImageProfile("eng+chi_sim", "manga-zh"),
            DeckNaming("Chinese Vocabulary", "zh", "kamishibai.json"),
        ),
    }
    if code in profiles:
        return profiles[code]
    raise ValueError(f"Unsupported target language '{code}'")


def _assets():
    """Return the traversable resource container for packaged assets."""
    return files("kamishibai.assets")


def _root():
    """Return the working directory that stores generated output."""
    return Path.cwd()


def _arguments(argv):
    """Parse CLI arguments for the unified kamishibai application."""
    parser = argparse.ArgumentParser(description="Convert vocabulary JSON to Anki deck")
    parser.add_argument("--deck", help="Custom deck name")
    parser.add_argument("path", nargs="?", help="Path to vocabulary JSON file")
    return parser.parse_args(argv)


def _text(name):
    """Load a packaged text asset by filename."""
    return (_assets() / name).read_text(encoding="utf-8").strip()


def _template():
    """Load the packaged manga template JSON document."""
    return json.loads((_assets() / "manga_template.json").read_text(encoding="utf-8"))


def _client():
    """Build a Gemini API client from GEMINI_API_KEY."""
    key = os.environ.get("GEMINI_API_KEY")
    if not key:
        raise ValueError("GEMINI_API_KEY environment variable is not set; export it before running")
    return genai.Client(api_key=key)


def _prefix(name):
    """Return a filesystem-friendly prefix derived from the deck name."""
    value = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")
    return value or "deck"


def _naming(args):
    """Return the effective deck naming after applying CLI overrides."""
    name = args.deck if args.deck else "Kamishibai Deck"
    return DeckNaming(name, _prefix(name), "kamishibai.json")


class _Fonts:
    """Selects a PDF font family for each processed entry."""

    def __init__(self):
        self._default = FontFamily("DejaVu Sans")
        self._cjk = FontFamily("Hiragino Sans GB")

    def selected(self, entry):
        """Return the report font family for a single entry."""
        if entry.get("target_lang") == "zh":
            return self._cjk
        return self._default


def _path(args):
    """Resolve the input vocabulary path from CLI arguments or default Downloads path."""
    default = os.path.expanduser("~/Downloads/kamishibai.json")
    return args.path if args.path else default


class _Media:
    """Builds per-target audio and illustration services lazily."""

    def __init__(self, client):
        self._client = client
        self._translator = SceneTranslator(client, _text("scene_prompt.txt"), _template())
        self._renderer = MangaRenderer(
            client,
            retries=3,
            text=TextDetectors(
                {code: TextDetector(60, _profile(code).imagery().ocr()) for code in ("de", "el", "en", "es", "zh")},
                TextDetector(60, "eng"),
            ),
            border=BorderDetector(width=6, brightness=240, margin=10),
        )
        self._audio = {}
        self._illustration = {}

    def audio(self, entry):
        """Return the audio service for the entry target language."""
        code = entry["target_lang"]
        if code not in self._audio:
            profile = _profile(code)
            self._audio[code] = Audio(
                self._client,
                Cache(profile.audio().cache()),
                profile.audio().prompt(),
                TtsVoice(("gemini-2.5-flash-preview-tts", "gemini-2.5-pro-preview-tts")),
            )
        return self._audio[code]

    def illustration(self, entry):
        """Return the illustration service for the entry target language."""
        code = entry["target_lang"]
        if code not in self._illustration:
            profile = _profile(code)
            self._illustration[code] = Illustration(
                Cache(profile.imagery().cache()),
                self._translator,
                self._renderer,
            )
        return self._illustration[code]


def main(argv=None):
    """Run the application logic for the provided CLI arguments."""
    args = _arguments(argv)
    vocabulary = Vocabulary(_path(args), VocabularyMapping())
    document = vocabulary.document()
    client = _client()
    naming = _naming(args)
    media = _Media(client)
    entries = vocabulary.entries(document)
    model = CardModel(StableId(f"{naming.name()} Model").value()).model()
    deck = genanki.Deck(StableId(naming.name()).value(), naming.name())
    container = VocabularyDeck(deck, VocabularyNote(model), [])
    progress = ProgressSelector(sys.stdout.isatty()).selected()
    failed, processed = Pipeline(media, media, container, progress).process(entries)
    output = _root() / "output"
    output.mkdir(exist_ok=True)
    stamp = datetime.now().strftime("%Y-%m-%d_%H%M%S")
    apkg = output / f"{naming.prefix()}_{stamp}.apkg"
    container.save(str(apkg))
    report = Report(VocabularyLayout(), _Fonts(), Thumbnail(150))
    for entry, imagepath in processed:
        report.append(entry, imagepath)
    pdf = output / f"{naming.prefix()}_{stamp}.pdf"
    report.save(str(pdf))
    progress.result("Anki deck", str(apkg))
    progress.result("Report", str(pdf))
    progress.result("Output", str(output))
    progress.finish(len(entries) - len(failed), len(entries), failed)


def run(argv=None):
    """Execute the CLI and translate failures into process exit codes."""
    try:
        main(sys.argv[1:] if argv is None else argv)
        return 0
    except KeyboardInterrupt:
        return 130
    except (FileNotFoundError, json.JSONDecodeError, ValueError, PermissionError, OSError, EnvironmentError) as error:
        diagnosis = DiagnosisSelector(sys.stderr.isatty()).selected()
        diagnosis.show(str(error), getattr(error, "filename", ""))
        return 1
