#!/usr/bin/env python3
"""
Convert vocabulary JSON to Anki deck with Russian sentences on front (Greek version)
"""

import hashlib
import json
import os
import random
import sys
import wave
from datetime import datetime

import genanki
from google import genai
from google.genai import types

from manga import BorderDetector
from manga import Cache
from manga import MangaRenderer
from manga import SceneTranslator
from manga import TextDetector


class AudioGenerator:
    """
    Generates audio files from text using Gemini TTS
    """

    def __init__(self, client, cache, prompt):
        self._client = client
        self._cache = cache
        self._prompt = prompt

    def generate(self, text):
        """
        Generate audio file from text and return tuple of filename and cached flag
        """
        digest = hashlib.md5(text.encode()).hexdigest()[:12]
        filename = f"{digest}.wav"
        if self._cache.exists(filename):
            return (filename, True)
        filepath = self._cache.filepath(filename)
        prompt = self._prompt.format(text=text)
        retries = 2
        for attempt in range(retries):
            try:
                response = self._client.models.generate_content(
                    model="gemini-2.5-flash-preview-tts",
                    contents=prompt,
                    config=types.GenerateContentConfig(
                        response_modalities=["AUDIO"],
                        speech_config=types.SpeechConfig(
                            voice_config=types.VoiceConfig(
                                prebuilt_voice_config=types.PrebuiltVoiceConfig(
                                    voice_name="Charon",
                                )
                            )
                        ),
                    ),
                )
                if not response.candidates:
                    print(f"Warning: No candidates in response for audio: {text}")
                    return (None, False)
                if not response.candidates[0].content:
                    print(f"Warning: No content in response for audio: {text}")
                    return (None, False)
                data = response.candidates[0].content.parts[0].inline_data.data
                with wave.open(filepath, "wb") as wf:
                    wf.setnchannels(1)
                    wf.setsampwidth(2)
                    wf.setframerate(24000)
                    wf.writeframes(data)
                return (filename, False)
            except Exception as error:
                print(f"Error generating audio: {error}")
                return (None, False)
        print(f"Failed to generate audio after {retries} attempts")
        return (None, False)


class ImageGenerator:
    """
    Generates manga images via two-step pipeline: sentence → scene → illustration
    """

    def __init__(self, client, cache, translator, renderer):
        self._client = client
        self._cache = cache
        self._translator = translator
        self._renderer = renderer

    def generate(self, sentence, word):
        """
        Generate manga image and return tuple of filename and cached flag
        """
        digest = hashlib.md5(sentence.encode()).hexdigest()[:12]
        filename = f"{digest}.jpg"
        if self._cache.exists(filename):
            return (filename, True)
        filepath = self._cache.filepath(filename)
        scene = self._translator.translate(sentence)
        image = self._renderer.render(scene, word)
        image.save(filepath, "JPEG", quality=60)
        return (filename, False)


class VocabularyDeck:
    """
    Represents an Anki deck for vocabulary
    """

    def __init__(self, path):
        self._path = path
        self._model = self._create()
        self._deck = genanki.Deck(
            random.randrange(1 << 30, 1 << 31), "Greek Vocabulary"
        )
        self._media = []

    def _create(self):
        """
        Create the Anki note model
        """
        return genanki.Model(
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
                    "qfmt": '<div style="text-align: center; padding: 20px;">{{Image}}<div style="font-size: 20px; margin-top: 15px;">{{RussianSentence}}</div></div>',
                    "afmt": '{{FrontSide}}<hr id="answer">{{Audio}}<div style="font-size: 22px; font-weight: bold; text-align: center; margin: 20px 0;">{{Example}}</div><div style="font-size: 13px; color: #aaa; margin-top: 4px;">{{PronunciationAll}}</div><div style="font-size: 17px; color: #ddd; margin-top: 15px;"><strong>{{Word}}</strong> {{Pronunciation}}</div><div style="font-size: 15px; color: #bbb; margin-top: 3px;">{{Translation}}</div><div style="font-size: 13px; color: #999; margin-top: 8px;">Importance: {{Importance}}/10</div><div style="font-size: 14px; color: #aaa; margin-top: 12px; padding: 10px; background-color: rgba(255,255,255,0.05); border-radius: 5px; text-align: left;">{{Context}}</div>',
                },
            ],
        )

    def add(
        self,
        sentence,
        word,
        pronunciation,
        translation,
        example,
        importance,
        audio,
        image,
        context,
        transcription,
    ):
        """
        Add a note to the deck
        """
        note = genanki.Note(
            model=self._model,
            fields=[
                sentence,
                word.lower(),
                pronunciation,
                translation,
                example,
                importance,
                audio,
                image,
                context.replace("\n", "<br>"),
                transcription,
            ],
        )
        self._deck.add_note(note)

    def attach(self, filepath):
        """
        Attach media file to the deck
        """
        self._media.append(filepath)

    def save(self, output):
        """
        Save the deck to a file
        """
        package = genanki.Package(self._deck)
        package.media_files = self._media
        package.write_to_file(output)


class JsonReader:
    """
    Reads vocabulary from JSON file
    """

    def __init__(self, path):
        self._path = path

    def read(self):
        """
        Read and return all vocabulary entries
        """
        with open(self._path, "r", encoding="utf-8") as file:
            data = json.load(file)
        entries = []
        for row in data:
            if row.get("word") and row.get("sentence_ru"):
                entries.append(
                    {
                        "word": row["word"],
                        "pronunciation": row.get("pronunciation", ""),
                        "translation": row.get("translation_ru", ""),
                        "example": row.get("sentence_el", ""),
                        "sentence": row["sentence_ru"],
                        "context": row.get("context_ru", ""),
                        "importance": str(row.get("importance", "")),
                        "transcription": row.get("pronunciation_all", ""),
                    }
                )
        return entries


def main():
    """
    Main function to convert JSON to Anki deck
    """
    key = os.environ.get("GEMINI_API_KEY")
    if not key:
        raise ValueError("GEMINI_API_KEY environment variable is not set")
    client = genai.Client(api_key=key)
    root = os.path.dirname(os.path.abspath(__file__))
    with open("greek_audio_prompt.txt", "r", encoding="utf-8") as f:
        prompt = f.read().strip()
    audio = AudioGenerator(client, Cache("greek_audio"), prompt)
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
    images = ImageGenerator(client, cache, translator, renderer)
    default = os.path.expanduser("~/Downloads/vocabulary_greek.json")
    path = sys.argv[1] if len(sys.argv) > 1 else default
    reader = JsonReader(path)
    entries = reader.read()
    deck = VocabularyDeck(path)
    failed = []
    for index, entry in enumerate(entries, 1):
        print(f"Processing card {index}/{len(entries)}: {entry['word']}")
        result = audio.generate(entry["example"])
        audiofile, cached = result
        if audiofile is None:
            print(f"  Skipping - no audio")
            failed.append({"word": entry["word"], "reason": "no audio"})
            continue
        status = "cached" if cached else "generated"
        result = images.generate(entry["example"], entry["word"])
        imagefile, cached = result
        if imagefile is None:
            print(f"  Skipping - no image")
            failed.append({"word": entry["word"], "reason": "no image"})
            continue
        tag = "cached" if cached else "generated"
        print(f"  [audio: {status}, image: {tag}]")
        audiopath = Cache("greek_audio").filepath(audiofile)
        deck.attach(audiopath)
        imagepath = Cache("greek_manga").filepath(imagefile)
        deck.attach(imagepath)
        html = f"<img src='{imagefile}' style='max-width: 512px; width: 100%; height: auto; border-radius: 10px;'>"
        deck.add(
            entry["sentence"],
            entry["word"],
            entry["pronunciation"],
            entry["translation"],
            entry["example"],
            entry["importance"],
            f"[sound:{audiofile}]",
            html,
            entry["context"],
            entry["transcription"],
        )
    output = os.path.join(root, "output")
    os.makedirs(output, exist_ok=True)
    output = os.path.join(output, f"greek_{datetime.now().strftime('%Y-%m-%d_%H%M%S')}.apkg")
    deck.save(output)
    successful = len(entries) - len(failed)
    print(f"\nCreated Anki deck with {successful}/{len(entries)} cards: {output}")
    if failed:
        print(f"\nSkipped {len(failed)} card(s):")
        for item in failed:
            print(f"  - {item['word']}: {item['reason']}")


if __name__ == "__main__":
    main()
