#!/usr/bin/env python3
"""
Convert vocabulary JSON to Anki deck with Russian sentences on front
"""

import json
import genanki
import hashlib
import os
import random
import sys
import wave
from io import BytesIO
from PIL import Image
from google import genai
from google.genai import types


class Cache:
    """
    Persistent file cache for generated media
    """

    def __init__(self, name):
        self._root = os.path.join(os.path.dirname(__file__), "cache")
        self._path = os.path.join(self._root, name)
        os.makedirs(self._path, exist_ok=True)

    def root(self):
        """
        Return root cache directory path
        """
        return self._root

    def path(self):
        """
        Return cache directory path
        """
        return self._path

    def exists(self, filename):
        """
        Check if file exists in cache
        """
        return os.path.exists(os.path.join(self._path, filename))

    def filepath(self, filename):
        """
        Return full path to cached file
        """
        return os.path.join(self._path, filename)


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
        models = ["gemini-2.5-flash-preview-tts", "gemini-2.5-pro-preview-tts"]
        for model in models:
            try:
                response = self._client.models.generate_content(
                    model=model,
                    contents=prompt,
                    config=types.GenerateContentConfig(
                        response_modalities=["AUDIO"],
                        speech_config=types.SpeechConfig(
                            voice_config=types.VoiceConfig(
                                prebuilt_voice_config=types.PrebuiltVoiceConfig(
                                    voice_name="Kore",
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
                if "RESOURCE_EXHAUSTED" in str(error):
                    print(f"Rate limited on {model}, falling back")
                    continue
                print(f"Error generating audio: {error}")
                return (None, False)
        print(f"Failed to generate audio on all models")
        return (None, False)


class ImageGenerator:
    """
    Generates images from text using Gemini 3 Pro Image
    """

    def __init__(self, client, cache, prompt):
        self._client = client
        self._cache = cache
        self._prompt = prompt

    def generate(self, text, context=""):
        """
        Generate image file from text and return tuple of filename and cached flag
        """
        combined = f"{text}{context}"
        digest = hashlib.md5(combined.encode()).hexdigest()[:12]
        filename = f"{digest}.jpg"
        if self._cache.exists(filename):
            return (filename, True)
        filepath = self._cache.filepath(filename)
        instruction = f" Keep in mind the context: {context}." if context else ""
        prompt = self._prompt.format(text=text, context=instruction)
        retries = 2
        for attempt in range(retries):
            try:
                response = self._client.models.generate_content(
                    model="gemini-3-pro-image-preview",
                    contents=[prompt],
                    config=types.GenerateContentConfig(
                        response_modalities=["IMAGE"],
                        image_config=types.ImageConfig(
                            aspect_ratio="1:1",
                        ),
                        safety_settings=[
                            types.SafetySetting(
                                category="HARM_CATEGORY_HARASSMENT",
                                threshold="BLOCK_NONE",
                            ),
                            types.SafetySetting(
                                category="HARM_CATEGORY_HATE_SPEECH",
                                threshold="BLOCK_NONE",
                            ),
                            types.SafetySetting(
                                category="HARM_CATEGORY_SEXUALLY_EXPLICIT",
                                threshold="BLOCK_NONE",
                            ),
                            types.SafetySetting(
                                category="HARM_CATEGORY_DANGEROUS_CONTENT",
                                threshold="BLOCK_NONE",
                            ),
                        ],
                    ),
                )
                if not response.candidates:
                    print(f"Warning: No candidates in response for image: {text}")
                    return (None, False)
                if not response.candidates[0].content:
                    print(f"Warning: No content in response for image: {text}")
                    return (None, False)
                for part in response.candidates[0].content.parts:
                    if part.inline_data is not None:
                        image = Image.open(BytesIO(part.inline_data.data))
                        gray = image.convert("L")
                        gray.save(filepath, "JPEG", quality=60)
                        return (filename, False)
                print(f"Warning: No image data found in response for image: {text}")
                return (None, False)
            except Exception as error:
                print(f"Error generating image: {error}")
                return (None, False)
        print(f"Failed to generate image after {retries} attempts")
        return (None, False)


class VocabularyDeck:
    """
    Represents an Anki deck for vocabulary
    """

    def __init__(self, path):
        self._path = path
        self._model = self._create()
        self._deck = genanki.Deck(
            random.randrange(1 << 30, 1 << 31), "English Vocabulary"
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
                {"name": "Hint"},
                {"name": "Context"},
            ],
            templates=[
                {
                    "name": "Card 1",
                    "qfmt": '<div style="text-align: center; padding: 20px;">{{Image}}<div style="font-size: 20px; margin-top: 15px;">{{RussianSentence}}</div><div style="font-size: 14px; color: #888; margin-top: 8px; font-style: italic;">{{Hint}}</div></div>',
                    "afmt": '{{FrontSide}}<hr id="answer">{{Audio}}<div style="font-size: 22px; font-weight: bold; text-align: center; margin: 20px 0;">{{Example}}</div><div style="font-size: 17px; color: #ddd; margin-top: 15px;"><strong>{{Word}}</strong> {{Pronunciation}}</div><div style="font-size: 15px; color: #bbb; margin-top: 3px;">{{Translation}}</div><div style="font-size: 13px; color: #999; margin-top: 8px;">Importance: {{Importance}}/10</div><div style="font-size: 14px; color: #aaa; margin-top: 12px; padding: 10px; background-color: rgba(255,255,255,0.05); border-radius: 5px;">{{Context}}</div>',
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
        highlight,
        hint,
        context,
    ):
        """
        Add a note to the deck
        """
        highlighted = sentence.replace(highlight, f"<strong><em>{highlight}</em></strong>") if highlight else sentence
        note = genanki.Note(
            model=self._model,
            fields=[
                highlighted,
                word.lower(),
                pronunciation,
                translation,
                example,
                importance,
                audio,
                image,
                hint,
                context,
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
                        "example": row.get("sentence_en", ""),
                        "sentence": row["sentence_ru"],
                        "highlight": row.get("highlight_ru", ""),
                        "hint": row.get("hint_ru", ""),
                        "context": row.get("context_ru", ""),
                        "importance": str(row.get("importance", "")),
                    }
                )
        return entries


def main():
    """
    Main function to convert CSV to Anki deck
    """
    key = os.environ.get("GEMINI_API_KEY")
    if not key:
        raise ValueError("GEMINI_API_KEY environment variable is not set")
    with open("audio_prompt.txt", "r", encoding="utf-8") as f:
        prompt = f.read().strip()
    cache = Cache("audio")
    print(f"Cache directory: {cache.root()}")
    client = genai.Client(api_key=key)
    audio = AudioGenerator(client, cache, prompt)
    with open("image_prompt.txt", "r", encoding="utf-8") as f:
        prompt = f.read().strip()
    cache = Cache("images")
    images = ImageGenerator(client, cache, prompt)
    default = os.path.expanduser("~/Downloads/vocabulary.json")
    path = sys.argv[1] if len(sys.argv) > 1 else default
    reader = JsonReader(path)
    entries = reader.read()
    deck = VocabularyDeck("/Users/chichikov/Downloads/vocabulary.csv")
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
        result = images.generate(entry["example"], entry["context"])
        imagefile, cached = result
        if imagefile is None:
            print(f"  Skipping - no image")
            failed.append({"word": entry["word"], "reason": "no image"})
            continue
        tag = "cached" if cached else "generated"
        print(f"  [audio: {status}, image: {tag}]")
        audiopath = Cache("audio").filepath(audiofile)
        deck.attach(audiopath)
        imagepath = Cache("images").filepath(imagefile)
        deck.attach(imagepath)
        html = f"<img src='{imagefile}' style='max-width: 100%; height: auto; border-radius: 10px;'>"
        deck.add(
            entry["sentence"],
            entry["word"],
            entry["pronunciation"],
            entry["translation"],
            entry["example"],
            entry["importance"],
            f"[sound:{audiofile}]",
            html,
            entry["highlight"],
            entry["hint"],
            entry["context"],
        )
    output = "/Users/chichikov/Downloads/cards.apkg"
    deck.save(output)
    successful = len(entries) - len(failed)
    print(f"\nCreated Anki deck with {successful}/{len(entries)} cards: {output}")
    if failed:
        print(f"\nSkipped {len(failed)} card(s):")
        for item in failed:
            print(f"  - {item['word']}: {item['reason']}")
        path = "/Users/chichikov/Downloads/vocabulary_failed.csv"
        with open(path, "w", encoding="utf-8") as f:
            f.write("word,reason\n")
            for item in failed:
                f.write(f"{item['word']},{item['reason']}\n")
        print(f"\nFailed cards saved to: {path}")


if __name__ == "__main__":
    main()
