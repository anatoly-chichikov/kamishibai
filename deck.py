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
class VocabularyNote:
    """Assembles vocabulary notes with highlight, hint, and optional transcription"""

    def __init__(self, model):
        self._model = model

    def note(self, entry, audio, image):
        """Assemble a genanki Note from entry dict"""
        sentence = entry["sentence"]
        highlight = entry["highlight"]
        highlighted = sentence.replace(highlight, f"<strong><em>{highlight}</em></strong>") if highlight else sentence
        return genanki.Note(
            model=self._model,
            fields=[
                highlighted,
                entry["word"].lower(),
                Transcription(entry["pronunciation"]).formatted(),
                entry["translation"],
                entry["example"],
                entry["importance"],
                audio,
                image,
                entry["hint"],
                HtmlLineBreaks(entry["context"]).formatted(),
                Transcription(entry.get("transcription", "")).formatted(),
            ],
        )


@final
class VocabularyMapping:
    """Maps vocabulary JSON rows to normalized entry dicts"""

    def __init__(self, required, example, transcription):
        self._required = required
        self._example = example
        self._transcription = transcription

    def mapped(self, row):
        """Return normalized entry dict or None if row is invalid"""
        for field in self._required:
            if not row.get(field):
                return None
        result = {
            "word": row["word"],
            "pronunciation": row.get("pronunciation") or "",
            "translation": row.get("translation_ru") or "",
            "example": row.get(self._example) or "" if self._example else "",
            "sentence": row["sentence_ru"],
            "highlight": row.get("highlight_ru") or "",
            "hint": row.get("hint_ru") or "",
            "context": row.get("context_ru") or "",
            "importance": str(row.get("importance") or ""),
        }
        if self._transcription:
            result["transcription"] = row.get(self._transcription) or ""
        return result


@final
class VocabularyLayout:
    """Formats vocabulary entries as text lines for PDF report"""

    def row(self, entry):
        """Return list of (text, font_size) tuples for a vocabulary entry"""
        pronunciation = entry.get("pronunciation", "")
        header = entry["word"]
        if pronunciation:
            header += f" /{pronunciation.strip('/')}/"
        header += f' — {entry["translation"]}'
        lines = [(header, 11)]
        example = entry.get("example", "")
        if example:
            lines.append((example, 9))
        sentence = entry.get("sentence", "")
        if sentence:
            lines.append((f"Перевод: {sentence}", 9))
        context = entry.get("context", "")
        if context:
            lines.append((f"Контекст: {context}", 8))
        hint = entry.get("hint", "")
        if hint:
            lines.append((f"Подсказка: {hint}", 8))
        importance = entry.get("importance", "")
        if importance:
            lines.append((f"Важность: {importance}/10", 8))
        return lines


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
class StableId:
    """Derives a deterministic genanki-compatible integer ID from a name"""

    def __init__(self, name):
        self._name = name

    def value(self):
        """Return a stable 31-bit integer derived from the name via SHA-256"""
        digest = hashlib.sha256(self._name.encode("utf-8")).hexdigest()
        return int(digest[:8], 16) % (1 << 31)


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
class FontFamily:
    """Resolves regular and bold variants of a font family via fc-match"""

    def __init__(self, family):
        self._regular = FontPath(family)
        self._bold = FontPath(f"{family}:Bold")

    def regular(self):
        """Return absolute path to the regular weight TTF file"""
        return self._regular.resolved()

    def bold(self):
        """Return absolute path to the bold weight TTF file"""
        return self._bold.resolved()


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
        pdf.add_font("dejavu", "", self._font.regular())
        pdf.add_font("dejavu", "B", self._font.bold())
        pdf.set_font("dejavu", size=10)
        pdf.add_page()
        with tempfile.TemporaryDirectory() as thumbdir:
            for entry, imagepath in self._rows:
                if pdf.get_y() > 240:
                    pdf.add_page()
                self._row(pdf, entry, imagepath, thumbdir)
        pdf.output(output)

    def _row(self, pdf, entry, imagepath, thumbdir):
        """Render a single entry row with optional image thumbnail"""
        top = pdf.get_y()
        page = pdf.page
        if imagepath and os.path.isfile(imagepath):
            thumb = self._thumbnail.compressed(imagepath, thumbdir)
            pdf.image(thumb, x=10, y=top, w=25, h=25)
        indent = 40
        width = pdf.w - indent - pdf.r_margin
        pdf.set_xy(indent, top)
        for idx, (text, size) in enumerate(self._layout.row(entry)):
            if idx == 0:
                pdf.set_font("dejavu", style="B", size=size)
                pdf.set_text_color(0, 0, 0)
            elif size <= 8:
                pdf.set_font("dejavu", style="", size=size)
                pdf.set_text_color(120, 120, 120)
            else:
                pdf.set_font("dejavu", style="", size=size)
                pdf.set_text_color(0, 0, 0)
            pdf.set_x(indent)
            pdf.multi_cell(w=width, h=size * 0.5, text=str(text), align="L")
        if pdf.page != page:
            top = pdf.t_margin
        bottom = max(pdf.get_y(), top + 25)
        pdf.set_y(bottom)
        pdf.ln(3)
        pdf.set_draw_color(200, 200, 200)
        pdf.line(10, pdf.get_y(), pdf.w - pdf.r_margin, pdf.get_y())
        pdf.ln(4)


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

    def filepath(self, filename):
        """Return full path to a cached audio file"""
        return self._cache.filepath(filename)

    def generate(self, text):
        """Generate audio file and return tuple of filename and cached flag"""
        if not text.strip():
            raise ValueError("Cannot generate audio for empty text")
        digest = hashlib.md5(text.encode()).hexdigest()[:12]
        filename = f"{digest}.wav"
        if self._cache.exists(filename):
            return (filename, True)
        data = self._speech(self._prompt.format(text=text), text)
        self._commit(data, filename)
        return (filename, False)

    def _speech(self, prompt, text):
        """Call TTS model with fallback and return raw audio bytes"""
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
                return response.candidates[0].content.parts[0].inline_data.data
            except Exception as error:
                if "RESOURCE_EXHAUSTED" in str(error):
                    continue
                raise
        raise ValueError(f"Failed to generate audio on all models for '{text}'")

    def _commit(self, data, filename):
        """Write WAV data to cache atomically"""
        staged = self._cache.stage(".wav")
        try:
            with wave.open(staged, "wb") as wf:
                wf.setnchannels(1)
                wf.setsampwidth(2)
                wf.setframerate(24000)
                wf.writeframes(data)
            self._cache.commit(staged, filename)
        except BaseException:
            if os.path.exists(staged):
                os.remove(staged)
            raise


@final
class Illustration:
    """Generates manga images via two-step pipeline"""

    def __init__(self, cache, translator, renderer):
        self._cache = cache
        self._translator = translator
        self._renderer = renderer

    def filepath(self, filename):
        """Return full path to a cached illustration file"""
        return self._cache.filepath(filename)

    def generate(self, sentence, word, progress):
        """Generate manga image and return tuple of filename and cached flag"""
        digest = hashlib.md5(sentence.encode()).hexdigest()[:12]
        filename = f"{digest}.jpg"
        scenefile = f"{digest}.json"
        path = self._cache.filepath(filename)
        if self._cache.exists(filename):
            self._cached(scenefile, progress)
            progress.done("Rendering manga", "cached", path)
            return (filename, True)
        scene = self._scene(sentence, scenefile, progress)
        progress.step("Rendering manga")
        image = self._renderer.render(scene, word, progress)
        self._commit(image, filename)
        progress.done("Rendering manga", "rendered", path)
        return (filename, False)

    def _cached(self, scenefile, progress):
        """Report scene cache status when image is already cached"""
        if self._cache.exists(scenefile):
            progress.done("Composing scene", "cached", self._cache.filepath(scenefile))
        else:
            progress.done("Composing scene", "cached")

    def _scene(self, sentence, scenefile, progress):
        """Load scene from cache or translate and cache it"""
        scenepath = self._cache.filepath(scenefile)
        progress.step("Composing scene")
        if self._cache.exists(scenefile):
            with open(scenepath, "r", encoding="utf-8") as handle:
                scene = json.load(handle)
            progress.done("Composing scene", "cached", scenepath)
            return scene
        scene = self._translator.translate(sentence)
        staged = self._cache.stage(".json")
        try:
            with open(staged, "w", encoding="utf-8") as handle:
                json.dump(scene, handle, indent=2, ensure_ascii=False)
            self._cache.commit(staged, scenefile)
        except BaseException:
            if os.path.exists(staged):
                os.remove(staged)
            raise
        progress.done("Composing scene", "translated", scenepath)
        return scene

    def _commit(self, image, filename):
        """Write image to cache atomically"""
        staged = self._cache.stage(".jpg")
        try:
            image.save(staged, "JPEG", quality=60)
            self._cache.commit(staged, filename)
        except BaseException:
            if os.path.exists(staged):
                os.remove(staged)
            raise


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
        if not isinstance(data, list):
            raise ValueError(
                f"Expected a JSON array in '{self._path}' but found {type(data).__name__}"
            )
        result = []
        for row in data:
            entry = self._mapping.mapped(row)
            if entry is not None:
                result.append(entry)
        if not result:
            raise ValueError(
                f"No valid entries found in '{self._path}'; each entry requires 'word' and 'sentence_ru'"
            )
        return result


@final
class VocabularyDeck:
    """Assembles notes and media into an Anki deck"""

    def __init__(self, deck, format, media):
        self._deck = deck
        self._format = format
        self._media = list(media)
        self._seen = set(media)

    def add(self, entry, audio, image):
        """Add a note to the deck using the format protocol"""
        note = self._format.note(entry, audio, image)
        self._deck.add_note(note)

    def attach(self, filepath):
        """Attach a media file to the deck, skipping duplicates"""
        if filepath not in self._seen:
            self._seen.add(filepath)
            self._media.append(filepath)

    def save(self, output):
        """Export deck to an .apkg file"""
        package = genanki.Package(self._deck)
        package.media_files = self._media
        package.write_to_file(output)


@final
class Pipeline:
    """Orchestrates audio and image generation for each entry"""

    def __init__(self, audio, illustration, deck, progress):
        self._audio = audio
        self._illustration = illustration
        self._deck = deck
        self._progress = progress

    def process(self, entries):
        """Process all entries and return tuple of failures and processed list"""
        failed = []
        processed = []
        for index, entry in enumerate(entries, 1):
            self._progress.card(index, len(entries), entry["word"])
            try:
                self._progress.step("Generating audio")
                audiofile, cached = self._audio.generate(entry["example"])
                audiopath = self._audio.filepath(audiofile)
                label = "cached" if cached else "generated"
                self._progress.done("Generating audio", label, audiopath)
            except Exception as error:
                self._progress.skip(entry["word"], str(error))
                failed.append({"word": entry["word"], "reason": str(error)})
                continue
            try:
                imagefile, cached = self._illustration.generate(
                    entry["example"], entry["word"], self._progress
                )
            except Exception as error:
                self._progress.skip(entry["word"], str(error))
                failed.append({"word": entry["word"], "reason": str(error)})
                continue
            self._deck.attach(audiopath)
            imagepath = self._illustration.filepath(imagefile)
            self._deck.attach(imagepath)
            processed.append((entry, imagepath))
            sound = f"[sound:{audiofile}]"
            style = "max-width: 100%; height: auto; border-radius: 10px"
            html = f"<img src='{imagefile}' style='{style}'>"
            self._deck.add(entry, sound, html)
        return (failed, processed)
