#!/usr/bin/env python3
"""
Convert vocabulary JSON to Anki deck with Russian sentences on front
"""

import argparse
import json
import os
import sys
from datetime import datetime

import genanki
from google import genai

from deck import Audio
from deck import CardModel
from deck import StableId
from deck import FontFamily
from deck import Illustration
from deck import Pipeline
from deck import Report
from deck import Thumbnail
from deck import TtsVoice
from deck import Vocabulary
from deck import VocabularyDeck
from deck import VocabularyLayout
from deck import VocabularyMapping
from deck import VocabularyNote
from language import AudioProfile
from language import DeckNaming
from language import ImageProfile
from language import Language
from manga import BorderDetector
from manga import Cache
from manga import MangaRenderer
from manga import SceneTranslator
from manga import TextDetector
from progress import ProgressSelector


def _language(code):
    """Build Language configuration for the given language code"""
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


def main():
    """Main function to convert JSON to Anki deck"""
    parser = argparse.ArgumentParser(description="Convert vocabulary JSON to Anki deck")
    parser.add_argument("--lang", choices=["en", "el"], default="en", help="Language (default: en)")
    parser.add_argument("--deck", help="Custom deck name (overrides language default)")
    parser.add_argument("path", nargs="?", help="Path to vocabulary JSON file")
    args = parser.parse_args()
    lang = _language(args.lang)
    key = os.environ.get("GEMINI_API_KEY")
    if not key:
        raise ValueError("GEMINI_API_KEY environment variable is not set; export it before running")
    client = genai.Client(api_key=key)
    root = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(root, lang.audio().prompt()), "r", encoding="utf-8") as f:
        prompt = f.read().strip()
    voice = TtsVoice(("gemini-2.5-flash-preview-tts", "gemini-2.5-pro-preview-tts"))
    audio = Audio(client, Cache(lang.audio().cache()), prompt, voice)
    with open(os.path.join(root, "scene_prompt.txt"), "r", encoding="utf-8") as f:
        prompt = f.read().strip()
    with open(os.path.join(root, "manga_template.json"), "r", encoding="utf-8") as f:
        template = json.load(f)
    translator = SceneTranslator(client, prompt, template)
    text = TextDetector(60, lang.imagery().ocr())
    border = BorderDetector(width=6, brightness=240, margin=10)
    renderer = MangaRenderer(client, retries=3, text=text, border=border)
    cache = Cache(lang.imagery().cache())
    images = Illustration(cache, translator, renderer)
    default = os.path.expanduser(f"~/Downloads/{lang.naming().default()}")
    path = args.path if args.path else default
    vocabulary = Vocabulary(path, lang.mapping())
    entries = vocabulary.entries()
    naming = lang.naming()
    if args.deck:
        naming = DeckNaming(args.deck, naming.prefix(), naming.default())
    model = CardModel(StableId(f"{naming.name()} Model").value()).model()
    note = VocabularyNote(model)
    deck = genanki.Deck(StableId(naming.name()).value(), naming.name())
    container = VocabularyDeck(deck, note, [])
    progress = ProgressSelector(sys.stdout.isatty()).selected()
    pipeline = Pipeline(audio, images, container, progress)
    failed, processed = pipeline.process(entries)
    output = os.path.join(root, "output")
    os.makedirs(output, exist_ok=True)
    stamp = datetime.now().strftime("%Y-%m-%d_%H%M%S")
    apkg = os.path.join(output, f"{naming.prefix()}_{stamp}.apkg")
    container.save(apkg)
    layout = VocabularyLayout()
    font = FontFamily("DejaVu Sans")
    report = Report(layout, font, Thumbnail(150))
    for entry, imagepath in processed:
        report.append(entry, imagepath)
    pdf = os.path.join(output, f"{naming.prefix()}_{stamp}.pdf")
    report.save(pdf)
    progress.result("Anki deck", apkg)
    progress.result("Report", pdf)
    progress.result("Output", output)
    successful = len(entries) - len(failed)
    progress.finish(successful, len(entries), failed)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
    except (FileNotFoundError, json.JSONDecodeError, ValueError,
            PermissionError, OSError, EnvironmentError) as error:
        from diagnosis import DiagnosisSelector
        diagnosis = DiagnosisSelector(sys.stderr.isatty()).selected()
        diagnosis.show(str(error), getattr(error, "filename", ""))
        sys.exit(1)
