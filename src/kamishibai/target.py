#!/usr/bin/env python3
"""Language profile objects for schema-driven deck generation."""

from typing import final


@final
class AudioProfile:
    """Audio generation configuration for a language."""

    def __init__(self, language, cache):
        self._language = language
        self._cache = cache

    def language(self):
        """Return display name for the language."""
        return self._language

    def cache(self):
        """Return cache directory name."""
        return self._cache


@final
class ImageProfile:
    """Image generation configuration for a language."""

    def __init__(self, ocr, cache):
        self._ocr = ocr
        self._cache = cache

    def ocr(self):
        """Return OCR language string for Tesseract."""
        return self._ocr

    def cache(self):
        """Return image cache directory name."""
        return self._cache


@final
class DeckNaming:
    """Deck naming and input filename defaults for a language."""

    def __init__(self, name, prefix, default):
        self._name = name
        self._prefix = prefix
        self._default = default

    def name(self):
        """Return deck display name."""
        return self._name

    def prefix(self):
        """Return output file prefix."""
        return self._prefix

    def default(self):
        """Return default input JSON filename."""
        return self._default


@final
class FontProfile:
    """Report font configuration for a language."""

    def __init__(self, report):
        self._report = report

    def report(self):
        """Return the report font family name."""
        return self._report


@final
class UiLabels:
    """User-facing labels for reports and other textual UI."""

    def __init__(self, sentence, context, hint, importance):
        self._sentence = sentence
        self._context = context
        self._hint = hint
        self._importance = importance

    def sentence(self):
        """Return label for the source sentence row."""
        return self._sentence

    def context(self):
        """Return label for the context row."""
        return self._context

    def hint(self):
        """Return label for the hint row."""
        return self._hint

    def importance(self):
        """Return label for the importance row."""
        return self._importance


@final
class LanguageProfile:
    """Composes runtime, naming, font, and UI settings for one language."""

    def __init__(self, code, audio, imagery, naming, font, labels):
        self._code = code
        self._audio = audio
        self._imagery = imagery
        self._naming = naming
        self._font = font
        self._labels = labels

    def code(self):
        """Return the language code."""
        return self._code

    def audio(self):
        """Return the audio configuration."""
        return self._audio

    def imagery(self):
        """Return the image configuration."""
        return self._imagery

    def naming(self):
        """Return the naming configuration."""
        return self._naming

    def font(self):
        """Return the font configuration."""
        return self._font

    def labels(self):
        """Return the UI labels."""
        return self._labels


@final
class Profiles:
    """Registry of supported language profiles and runtime fallbacks."""

    def __init__(self, items, fallback):
        self._items = dict(items)
        self._fallback = fallback

    def item(self, code):
        """Return the configured profile for the given language code."""
        if code in self._items:
            return self._items[code]
        raise ValueError(f"Unsupported language '{code}'")

    def codes(self):
        """Return supported language codes."""
        return tuple(self._items)

    def fallback_ocr(self):
        """Return the fallback OCR language string."""
        return self._fallback
