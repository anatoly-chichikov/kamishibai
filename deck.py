#!/usr/bin/env python3
"""
Shared module for vocabulary Anki deck generation
"""

import hashlib
import json
import os
import subprocess
import tempfile
import wave
from typing import Protocol, final

import genanki
from fpdf import FPDF
from google.genai import types
from PIL import Image

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


class ReportLayout(Protocol):
    """Protocol for formatting entry text lines in a PDF report"""

    def row(self, entry):
        """Return list of (text, font_size) tuples for a single report entry"""
        ...


@final
class Transcription:
    """Wraps a phonetic transcription in standard slash notation"""

    def __init__(self, value):
        self._value = value

    def formatted(self):
        """Return transcription wrapped in slashes or empty string if blank"""
        stripped = self._value.strip("/")
        return f"/{stripped}/" if stripped else ""


@final
class HtmlLineBreaks:
    """Converts newlines to HTML line breaks"""

    def __init__(self, value):
        self._value = value

    def formatted(self):
        """Return text with newlines replaced by br tags"""
        return self._value.replace("\n", "<br>") if self._value else ""


@final
class CardModel:
    """Unified Anki card model for vocabulary decks"""

    def __init__(self, identifier):
        self._identifier = identifier

    def model(self):
        """Return genanki Model with 11-field vocabulary template"""
        return genanki.Model(
            self._identifier,
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
                {"name": "PronunciationAll"},
            ],
            templates=[
                {
                    "name": "Card 1",
                    "qfmt": (
                        '<div style="max-width: 600px; margin: 0 auto; text-align: center; padding: 20px;">'
                        "{{Image}}"
                        '<div style="font-size: 20px; margin-top: 15px;">{{RussianSentence}}</div>'
                        "{{#Hint}}"
                        '<div style="font-size: 14px; color: #888; margin-top: 8px; font-style: italic;">{{Hint}}</div>'
                        "{{/Hint}}"
                        "</div>"
                    ),
                    "afmt": (
                        '{{FrontSide}}<hr id="answer">'
                        '<div style="max-width: 600px; margin: 0 auto; text-align: center;">'
                        "{{Audio}}"
                        '<div style="font-size: 22px; font-weight: bold; text-align: center; margin: 20px 0 4px 0;">{{Example}}</div>'
                        "{{#PronunciationAll}}"
                        '<div style="font-size: 13px; color: #aaa; margin-top: 4px;">{{PronunciationAll}}</div>'
                        "{{/PronunciationAll}}"
                        '<div style="font-size: 17px; margin-top: 15px;"><strong style="color: #ddd;">{{Word}}</strong> <span style="color: #aaa;">{{Pronunciation}}</span></div>'
                        '<div style="font-size: 15px; color: #bbb; margin-top: 3px;">{{Translation}}</div>'
                        '<div style="font-size: 13px; color: #999; margin-top: 8px;">Importance: {{Importance}}/10</div>'
                        '<div style="font-size: 14px; color: #aaa; margin-top: 12px; padding: 10px; background-color: rgba(255,255,255,0.05); border-radius: 5px; display: inline-block; text-align: left;">{{Context}}</div>'
                        "</div>"
                    ),
                },
            ],
        )


@final
class FontPath:
    """Resolves a font family name to a filesystem path via fc-match"""

    def __init__(self, family):
        self._family = family

    def resolved(self):
        """Return absolute path to the TTF file for the configured family"""
        result = subprocess.run(
            ["fc-match", "-f", "%{file}", self._family],
            capture_output=True,
            text=True,
        )
        path = result.stdout.strip()
        if not path or not os.path.isfile(path):
            raise FileNotFoundError(
                f"Font '{self._family}' not found via fc-match"
            )
        return path


@final
class Thumbnail:
    """Resizes an image to a target pixel size for PDF embedding"""

    def __init__(self, pixels):
        self._pixels = pixels

    def compressed(self, source, directory):
        """Return path to a resized JPEG copy in the given directory"""
        image = Image.open(source)
        image.thumbnail((self._pixels, self._pixels))
        name = f"thumb_{os.path.basename(source)}"
        result = os.path.join(directory, name)
        image.save(result, "JPEG", quality=60)
        return result


@final
class Report:
    """Accumulates vocabulary entries and renders a PDF report"""

    def __init__(self, layout, font, thumbnail):
        self._layout = layout
        self._font = font
        self._thumbnail = thumbnail
        self._rows = []

    def append(self, entry, imagepath):
        """Record an entry with its image path for later rendering"""
        self._rows.append((entry, imagepath))

    def save(self, output):
        """Render all accumulated entries to a PDF file"""
        pdf = FPDF()
        pdf.set_auto_page_break(auto=True, margin=15)
        pdf.add_font("dejavu", "", self._font.resolved())
        pdf.set_font("dejavu", size=10)
        pdf.add_page()
        with tempfile.TemporaryDirectory() as thumbdir:
            for entry, imagepath in self._rows:
                if pdf.get_y() > 260:
                    pdf.add_page()
                top = pdf.get_y()
                if imagepath and os.path.isfile(imagepath):
                    thumb = self._thumbnail.compressed(imagepath, thumbdir)
                    pdf.image(thumb, x=10, y=top, w=25, h=25)
                indent = 40
                pdf.set_xy(indent, top)
                for text, size in self._layout.row(entry):
                    pdf.set_font_size(size)
                    pdf.set_x(indent)
                    pdf.cell(w=0, h=size * 0.6, text=str(text))
                    pdf.ln(size * 0.6)
                pdf.ln(4)
        pdf.output(output)


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

    def __init__(self, audio, illustration, deck, report=None):
        self._audio = audio
        self._illustration = illustration
        self._deck = deck
        self._report = report

    def process(self, entries):
        """Process all entries and return list of failures"""
        failed = []
        for index, entry in enumerate(entries, 1):
            print(f"Processing card {index}/{len(entries)}: {entry['word']}")
            try:
                audiofile, cached = self._audio.generate(entry["example"])
            except Exception as error:
                print(f"  Skipping - {error}")
                failed.append({"word": entry["word"], "reason": str(error)})
                continue
            status = "cached" if cached else "generated"
            try:
                imagefile, cached = self._illustration.generate(
                    entry["example"], entry["word"]
                )
            except Exception as error:
                print(f"  Skipping - {error}")
                failed.append({"word": entry["word"], "reason": str(error)})
                continue
            tag = "cached" if cached else "generated"
            print(f"  [audio: {status}, image: {tag}]")
            audiopath = self._audio._cache.filepath(audiofile)
            self._deck.attach(audiopath)
            imagepath = self._illustration._cache.filepath(imagefile)
            self._deck.attach(imagepath)
            if self._report is not None:
                self._report.append(entry, imagepath)
            sound = f"[sound:{audiofile}]"
            html = f"<img src='{imagefile}' style='{_IMG_STYLE}'>"
            self._deck.add(entry, sound, html)
        return failed
