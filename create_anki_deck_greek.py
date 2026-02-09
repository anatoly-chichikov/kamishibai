#!/usr/bin/env python3
"""
Convert vocabulary JSON to Anki deck with Russian sentences on front (Greek version)
"""

import json
import os
import random
import sys
from datetime import datetime
from typing import final

import genanki
from google import genai

from deck import Audio
from deck import FieldMapping
from deck import Illustration
from deck import NoteFormat
from deck import Pipeline
from deck import TtsVoice
from deck import Vocabulary
from deck import VocabularyDeck
from manga import BorderDetector
from manga import Cache
from manga import MangaRenderer
from manga import SceneTranslator
from manga import TextDetector


@final
class GreekNote:
    """Assembles Greek vocabulary notes with transcription"""

    def __init__(self, model):
        self._model = model

    def note(self, entry, audio, image):
        """Assemble a genanki Note from entry dict"""
        return genanki.Note(
            model=self._model,
            fields=[
                entry["sentence"],
                entry["word"].lower(),
                entry["pronunciation"],
                entry["translation"],
                entry["example"],
                entry["importance"],
                audio,
                image,
                entry["context"].replace("\n", "<br>"),
                entry["transcription"],
            ],
        )


@final
class GreekMapping:
    """Maps Greek vocabulary JSON rows to normalized entry dicts"""

    def __init__(self, required):
        self._required = required

    def mapped(self, row):
        """Return normalized entry dict or None if row is invalid"""
        for field in self._required:
            if not row.get(field):
                return None
        return {
            "word": row["word"],
            "pronunciation": row.get("pronunciation", ""),
            "translation": row.get("translation_ru", ""),
            "example": row.get("sentence_el", ""),
            "sentence": row["sentence_ru"],
            "context": row.get("context_ru", ""),
            "importance": str(row.get("importance", "")),
            "transcription": row.get("pronunciation_all", ""),
        }


def main():
    """Main function to convert JSON to Anki deck"""
    key = os.environ.get("GEMINI_API_KEY")
    if not key:
        raise ValueError("GEMINI_API_KEY environment variable is not set")
    client = genai.Client(api_key=key)
    root = os.path.dirname(os.path.abspath(__file__))
    with open("greek_audio_prompt.txt", "r", encoding="utf-8") as f:
        prompt = f.read().strip()
    voice = TtsVoice(
        "Charon",
        ("gemini-2.5-flash-preview-tts", "gemini-2.5-pro-preview-tts"),
    )
    audio = Audio(client, Cache("greek_audio"), prompt, voice)
    with open(os.path.join(root, "scene_prompt.txt"), "r") as f:
        prompt = f.read().strip()
    with open(os.path.join(root, "manga_template.json"), "r") as f:
        template = json.load(f)
    translator = SceneTranslator(client, prompt, template)
    text = TextDetector(60)
    border = BorderDetector(width=6, brightness=240, margin=10)
    renderer = MangaRenderer(client, retries=3, text=text, border=border)
    cache = Cache("greek_manga")
    print(f"Cache directory: {cache.root()}")
    images = Illustration(cache, translator, renderer)
    default = os.path.expanduser("~/Downloads/vocabulary_greek.json")
    path = sys.argv[1] if len(sys.argv) > 1 else default
    mapping = GreekMapping(("word", "sentence_ru"))
    vocabulary = Vocabulary(path, mapping)
    entries = vocabulary.entries()
    model = genanki.Model(
        random.randrange(1 << 30, 1 << 31),
        "Vocabulary Model",
        fields=[
            {"name": "RussianSentence"},
            {"name": "Word"},
            {"name": "Pronunciation"},
            {"name": "Translation"},
            {"name": "Example"},
            {"name": "Importance"},
            {"name": "Audio"},
            {"name": "Image"},
            {"name": "Context"},
            {"name": "PronunciationAll"},
        ],
        templates=[
            {
                "name": "Card 1",
                "qfmt": '<div style="max-width: 600px; margin: 0 auto; text-align: center; padding: 20px;">{{Image}}<div style="font-size: 20px; margin-top: 15px;">{{RussianSentence}}</div></div>',
                "afmt": '{{FrontSide}}<hr id="answer"><div style="max-width: 600px; margin: 0 auto;">{{Audio}}<div style="font-size: 22px; font-weight: bold; text-align: center; margin: 20px 0;">{{Example}}</div><div style="font-size: 13px; color: #aaa; margin-top: 4px;">{{PronunciationAll}}</div><div style="font-size: 17px; color: #ddd; margin-top: 15px;"><strong>{{Word}}</strong> {{Pronunciation}}</div><div style="font-size: 15px; color: #bbb; margin-top: 3px;">{{Translation}}</div><div style="font-size: 13px; color: #999; margin-top: 8px;">Importance: {{Importance}}/10</div><div style="font-size: 14px; color: #aaa; margin-top: 12px; padding: 10px; background-color: rgba(255,255,255,0.05); border-radius: 5px; text-align: left;">{{Context}}</div></div>',
            },
        ],
    )
    format = GreekNote(model)
    deck = genanki.Deck(
        random.randrange(1 << 30, 1 << 31), "Greek Vocabulary"
    )
    container = VocabularyDeck(deck, format, [])
    pipeline = Pipeline(audio, images, container)
    failed = pipeline.process(entries)
    output = os.path.join(root, "output")
    os.makedirs(output, exist_ok=True)
    output = os.path.join(output, f"greek_{datetime.now().strftime('%Y-%m-%d_%H%M%S')}.apkg")
    container.save(output)
    successful = len(entries) - len(failed)
    print(f"\nCreated Anki deck with {successful}/{len(entries)} cards: {output}")
    if failed:
        print(f"\nSkipped {len(failed)} card(s):")
        for item in failed:
            print(f"  - {item['word']}: {item['reason']}")


if __name__ == "__main__":
    main()
