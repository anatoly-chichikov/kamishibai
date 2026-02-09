#!/usr/bin/env python3
"""
Shared module for vocabulary Anki deck generation
"""

import hashlib
import json
import os
import wave
from typing import Protocol, final

import genanki
from google.genai import types

from manga import Cache

_IMG_STYLE = "max-width: 100%; height: auto; border-radius: 10px"


class NoteFormat(Protocol):
    """Protocol for assembling a genanki Note from entry dict"""

    def note(self, entry, audio, image):
        """Assemble and return a genanki Note"""
        ...


class FieldMapping(Protocol):
    """Protocol for mapping JSON row to normalized entry dict"""

    def mapped(self, row):
        """Return normalized entry dict or None if row is invalid"""
        ...


@final
class TtsVoice:
    """Represents a TTS voice configuration with fallback models"""

    def __init__(self, name, models):
        self._name = name
        self._models = models

    def speech(self):
        """Return SpeechConfig for this voice"""
        return types.SpeechConfig(
            voice_config=types.VoiceConfig(
                prebuilt_voice_config=types.PrebuiltVoiceConfig(
                    voice_name=self._name,
                )
            )
        )

    def models(self):
        """Return tuple of model names for fallback iteration"""
        return self._models


@final
class Audio:
    """Generates audio files from text using Gemini TTS"""

    def __init__(self, client, cache, prompt, voice):
        self._client = client
        self._cache = cache
        self._prompt = prompt
        self._voice = voice

    def generate(self, text):
        """Generate audio file and return tuple of filename and cached flag"""
        digest = hashlib.md5(text.encode()).hexdigest()[:12]
        filename = f"{digest}.wav"
        if self._cache.exists(filename):
            return (filename, True)
        filepath = self._cache.filepath(filename)
        prompt = self._prompt.format(text=text)
        for model in self._voice.models():
            try:
                response = self._client.models.generate_content(
                    model=model,
                    contents=prompt,
                    config=types.GenerateContentConfig(
                        response_modalities=["AUDIO"],
                        speech_config=self._voice.speech(),
                    ),
                )
                if not response.candidates:
                    raise ValueError(f"No candidates in audio response for '{text}'")
                if not response.candidates[0].content:
                    raise ValueError(f"No content in audio response for '{text}'")
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
                raise
        raise ValueError(f"Failed to generate audio on all models for '{text}'")


@final
class Illustration:
    """Generates manga images via two-step pipeline"""

    def __init__(self, cache, translator, renderer):
        self._cache = cache
        self._translator = translator
        self._renderer = renderer

    def generate(self, sentence, word):
        """Generate manga image and return tuple of filename and cached flag"""
        digest = hashlib.md5(sentence.encode()).hexdigest()[:12]
        filename = f"{digest}.jpg"
        if self._cache.exists(filename):
            return (filename, True)
        filepath = self._cache.filepath(filename)
        scene = self._translator.translate(sentence)
        image = self._renderer.render(scene, word)
        image.save(filepath, "JPEG", quality=60)
        return (filename, False)


@final
class Vocabulary:
    """Reads vocabulary entries from a JSON file"""

    def __init__(self, path, mapping):
        self._path = path
        self._mapping = mapping

    def entries(self):
        """Load, filter, and return vocabulary entries"""
        with open(self._path, "r", encoding="utf-8") as file:
            data = json.load(file)
        result = []
        for row in data:
            entry = self._mapping.mapped(row)
            if entry is not None:
                result.append(entry)
        return result


@final
class VocabularyDeck:
    """Assembles notes and media into an Anki deck"""

    def __init__(self, deck, format, media):
        self._deck = deck
        self._format = format
        self._media = media

    def add(self, entry, audio, image):
        """Add a note to the deck using the format protocol"""
        note = self._format.note(entry, audio, image)
        self._deck.add_note(note)

    def attach(self, filepath):
        """Attach a media file to the deck"""
        self._media.append(filepath)

    def save(self, output):
        """Export deck to an .apkg file"""
        package = genanki.Package(self._deck)
        package.media_files = self._media
        package.write_to_file(output)


@final
class Pipeline:
    """Orchestrates audio and image generation for each entry"""

    def __init__(self, audio, illustration, deck):
        self._audio = audio
        self._illustration = illustration
        self._deck = deck

    def process(self, entries):
        """Process all entries and return list of failures"""
        failed = []
        for index, entry in enumerate(entries, 1):
            print(f"Processing card {index}/{len(entries)}: {entry['word']}")
            try:
                audiofile, cached = self._audio.generate(entry["example"])
            except ValueError as error:
                print(f"  Skipping - {error}")
                failed.append({"word": entry["word"], "reason": str(error)})
                continue
            status = "cached" if cached else "generated"
            try:
                imagefile, cached = self._illustration.generate(
                    entry["example"], entry["word"]
                )
            except ValueError as error:
                print(f"  Skipping - {error}")
                failed.append({"word": entry["word"], "reason": str(error)})
                continue
            tag = "cached" if cached else "generated"
            print(f"  [audio: {status}, image: {tag}]")
            audiopath = self._audio._cache.filepath(audiofile)
            self._deck.attach(audiopath)
            imagepath = self._illustration._cache.filepath(imagefile)
            self._deck.attach(imagepath)
            sound = f"[sound:{audiofile}]"
            html = f"<img src='{imagefile}' style='{_IMG_STYLE}'>"
            self._deck.add(entry, sound, html)
        return failed
