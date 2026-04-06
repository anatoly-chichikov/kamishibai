#!/usr/bin/env python3
"""
Language-specific configuration for vocabulary deck generation
"""

from typing import Protocol, final


class Mapping(Protocol):
    """Protocol for language-specific vocabulary field mapping"""

    def mapped(self, row):
        """Return normalized entry dict or None if row is invalid"""
        ...


@final
class AudioProfile:
    """Audio generation configuration for a language"""

    def __init__(self, prompt, cache):
        self._prompt = prompt
        self._cache = cache

    def prompt(self):
        """Return audio prompt filename"""
        return self._prompt

    def cache(self):
        """Return cache directory name"""
        return self._cache


@final
class ImageProfile:
    """Image generation configuration for a language"""

    def __init__(self, ocr, cache):
        self._ocr = ocr
        self._cache = cache

    def ocr(self):
        """Return OCR language string for Tesseract"""
        return self._ocr

    def cache(self):
        """Return image cache directory name"""
        return self._cache


@final
class DeckNaming:
    """Deck naming and output configuration for a language"""

    def __init__(self, name, prefix, default):
        self._name = name
        self._prefix = prefix
        self._default = default

    def name(self):
        """Return deck display name"""
        return self._name

    def prefix(self):
        """Return output file prefix"""
        return self._prefix

    def default(self):
        """Return default input JSON filename"""
        return self._default


@final
class Language:
    """Composes language-specific profiles and mapping"""

    def __init__(self, audio, imagery, naming, mapping):
        self._audio = audio
        self._imagery = imagery
        self._naming = naming
        self._mapping = mapping

    def audio(self):
        """Return AudioProfile for this language"""
        return self._audio

    def imagery(self):
        """Return ImageProfile for this language"""
        return self._imagery

    def naming(self):
        """Return DeckNaming for this language"""
        return self._naming

    def mapping(self):
        """Return VocabularyMapping for this language"""
        return self._mapping
