#!/usr/bin/env python3
"""Target-specific runtime configuration for deck generation."""

from typing import final


@final
class AudioProfile:
    """Audio generation configuration for a target language."""

    def __init__(self, language, cache):
        self._language = language
        self._cache = cache

    def language(self):
        """Return display name for the target language."""
        return self._language

    def cache(self):
        """Return cache directory name."""
        return self._cache


@final
class ImageProfile:
    """Image generation configuration for a target language."""

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
    """Deck naming and output configuration for a target language."""

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
class TargetProfile:
    """Composes runtime profiles for a supported target language."""

    def __init__(self, code, audio, imagery, naming):
        self._code = code
        self._audio = audio
        self._imagery = imagery
        self._naming = naming

    def code(self):
        """Return the target language code."""
        return self._code

    def audio(self):
        """Return AudioProfile for this target language."""
        return self._audio

    def imagery(self):
        """Return ImageProfile for this target language."""
        return self._imagery

    def naming(self):
        """Return DeckNaming for this target language."""
        return self._naming
