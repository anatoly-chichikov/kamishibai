#!/usr/bin/env python3
"""Unified command line application for deck generation."""

import argparse
import json
import os
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
from .language import AudioProfile
from .language import DeckNaming
from .language import ImageProfile
from .language import Language
from .manga import BorderDetector
from .manga import Cache
from .manga import MangaRenderer
from .manga import SceneTranslator
from .manga import TextDetector
from .progress import ProgressSelector


def _language(code):
    """Build the configured Language object for a supported code."""
    if code == "el":
        return Language(
            AudioProfile("greek_audio_prompt.txt", "greek_audio"),
            ImageProfile("eng+ell", "greek_manga"),
            DeckNaming("Greek Vocabulary", "greek", "vocabulary_greek.json"),
            VocabularyMapping(("word", "sentence_ru"), "sentence_el"),
        )
    return Language(
        AudioProfile("audio_prompt.txt", "audio"),
        ImageProfile("eng", "manga"),
        DeckNaming("English Vocabulary", "cards", "vocabulary.json"),
        VocabularyMapping(("word", "sentence_ru"), "sentence_en"),
    )


def _assets():
    """Return the traversable resource container for packaged assets."""
    return files("kamishibai.assets")


def _root():
    """Return the working directory that stores generated output."""
    return Path.cwd()


def _arguments(argv):
    """Parse CLI arguments for the unified kamishibai application."""
    parser = argparse.ArgumentParser(description="Convert vocabulary JSON to Anki deck")
    parser.add_argument("--lang", choices=["en", "el"], default="en", help="Language (default: en)")
    parser.add_argument("--deck", help="Custom deck name (overrides language default)")
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


def _naming(args, lang):
    """Return the effective deck naming after applying CLI overrides."""
    naming = lang.naming()
    if args.deck:
        return DeckNaming(args.deck, naming.prefix(), naming.default())
    return naming


def _path(args, lang):
    """Resolve the input vocabulary path from CLI arguments or default Downloads path."""
    default = os.path.expanduser(f"~/Downloads/{lang.naming().default()}")
    return args.path if args.path else default


def main(argv=None):
    """Run the application logic for the provided CLI arguments."""
    args = _arguments(argv)
    lang = _language(args.lang)
    client = _client()
    naming = _naming(args, lang)
    audio = Audio(
        client,
        Cache(lang.audio().cache()),
        _text(lang.audio().prompt()),
        TtsVoice(("gemini-2.5-flash-preview-tts", "gemini-2.5-pro-preview-tts")),
    )
    translator = SceneTranslator(client, _text("scene_prompt.txt"), _template())
    renderer = MangaRenderer(
        client,
        retries=3,
        text=TextDetector(60, lang.imagery().ocr()),
        border=BorderDetector(width=6, brightness=240, margin=10),
    )
    images = Illustration(Cache(lang.imagery().cache()), translator, renderer)
    entries = Vocabulary(_path(args, lang), lang.mapping()).entries()
    model = CardModel(StableId(f"{naming.name()} Model").value()).model()
    deck = genanki.Deck(StableId(naming.name()).value(), naming.name())
    container = VocabularyDeck(deck, VocabularyNote(model), [])
    progress = ProgressSelector(sys.stdout.isatty()).selected()
    failed, processed = Pipeline(audio, images, container, progress).process(entries)
    output = _root() / "output"
    output.mkdir(exist_ok=True)
    stamp = datetime.now().strftime("%Y-%m-%d_%H%M%S")
    apkg = output / f"{naming.prefix()}_{stamp}.apkg"
    container.save(str(apkg))
    report = Report(VocabularyLayout(), FontFamily("DejaVu Sans"), Thumbnail(150))
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
