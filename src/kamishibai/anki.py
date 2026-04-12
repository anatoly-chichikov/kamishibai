"""Anki deck assembly for kamishibai."""

import hashlib
from typing import Protocol, final

import genanki


class NoteFormat(Protocol):
    """Protocol for assembling a genanki Note from entry dict."""

    def note(self, entry, audio, image):
        """Assemble and return a genanki Note."""
        ...


@final
class Transcription:
    """Wraps a phonetic transcription in standard slash notation."""

    def __init__(self, value):
        self._value = value

    def formatted(self):
        """Return transcription wrapped in slashes or empty string if blank."""
        stripped = self._value.strip("/")
        return f"/{stripped}/" if stripped else ""


@final
class HtmlLineBreaks:
    """Converts newlines to HTML line breaks."""

    def __init__(self, value):
        self._value = value

    def formatted(self):
        """Return text with newlines replaced by br tags."""
        return self._value.replace("\n", "<br>") if self._value else ""


@final
class StableId:
    """Derives a deterministic genanki-compatible integer ID from a name."""

    def __init__(self, name):
        self._name = name

    def value(self):
        """Return a stable 31-bit integer derived from the name via SHA-256."""
        digest = hashlib.sha256(self._name.encode("utf-8")).hexdigest()
        return int(digest[:8], 16) % (1 << 31)


@final
class VocabularyNote:
    """Assembles vocabulary notes with highlight and optional transcription."""

    def __init__(self, model):
        self._model = model

    def note(self, entry, audio, image):
        """Assemble a genanki Note from entry dict."""
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
                HtmlLineBreaks(entry["example"]).formatted(),
                entry["importance"],
                audio,
                image,
                entry["hint"],
                HtmlLineBreaks(entry["context"]).formatted(),
                Transcription(entry.get("transcription", "")).formatted(),
            ],
        )


@final
class CardModel:
    """Unified Anki card model for vocabulary decks."""

    def __init__(self, identifier):
        self._identifier = identifier

    def model(self):
        """Return genanki Model with 11-field vocabulary template."""
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
                        '<div style="max-width: 600px; margin: 0 auto; text-align: center; padding: 0 20px;">'
                        "{{Audio}}"
                        '<div style="font-size: 22px; font-weight: bold; margin: 20px 0 4px 0;">{{Example}}</div>'
                        "{{#PronunciationAll}}"
                        '<div style="font-size: 13px; color: #aaa; margin-top: 4px;">{{PronunciationAll}}</div>'
                        "{{/PronunciationAll}}"
                        '<div style="font-size: 17px; margin-top: 15px;"><strong style="color: #ddd;">{{Word}}</strong> <span style="color: #aaa;">{{Pronunciation}}</span></div>'
                        '<div style="font-size: 15px; color: #bbb; margin-top: 3px;">{{Translation}}</div>'
                        '<div style="font-size: 13px; color: #999; margin-top: 8px;">Importance: {{Importance}}/10</div>'
                        "{{#Context}}"
                        '<div style="font-size: 14px; color: #aaa; margin-top: 12px; padding: 10px; background-color: rgba(255,255,255,0.05); border-radius: 5px; text-align: left;">{{Context}}</div>'
                        "{{/Context}}"
                        "</div>"
                    ),
                },
            ],
        )


@final
class VocabularyDeck:
    """Assembles notes and media into an Anki deck."""

    def __init__(self, deck, format, media):
        self._deck = deck
        self._format = format
        self._media = list(media)
        self._seen = set(media)

    def add(self, entry, audio, image):
        """Add a note to the deck using the format protocol."""
        note = self._format.note(entry, audio, image)
        self._deck.add_note(note)

    def attach(self, filepath):
        """Attach a media file to the deck, skipping duplicates."""
        if filepath not in self._seen:
            self._seen.add(filepath)
            self._media.append(filepath)

    def save(self, output):
        """Export deck to an .apkg file."""
        package = genanki.Package(self._deck)
        package.media_files = self._media
        package.write_to_file(output)
