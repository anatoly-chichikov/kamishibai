#!/usr/bin/env python3
"""
Unit tests for language configuration classes
"""

import uuid

import pytest

from kamishibai.deck import VocabularyMapping
from kamishibai.language import AudioProfile
from kamishibai.language import DeckNaming
from kamishibai.language import ImageProfile
from kamishibai.language import Language


class TestAudioProfileReturnsConfiguredValues:
    """AudioProfile returns the configured prompt and cache values"""

    def test_returns_prompt_filename(self):
        filename = f"prompt_{uuid.uuid4().hex[:6]}.txt"
        profile = AudioProfile(filename, "audio")
        assert profile.prompt() == filename, "prompt filename was not returned"

    def test_returns_cache_directory(self):
        cache = f"cache_{uuid.uuid4().hex[:6]}"
        profile = AudioProfile("prompt.txt", cache)
        assert profile.cache() == cache, "cache directory was not returned"


class TestImageProfileReturnsConfiguredValues:
    """ImageProfile returns the configured OCR and cache values"""

    def test_returns_ocr_language(self):
        ocr = f"eng+ell_{uuid.uuid4().hex[:4]}"
        profile = ImageProfile(ocr, "manga")
        assert profile.ocr() == ocr, "OCR language was not returned"

    def test_returns_cache_directory(self):
        cache = f"cache_{uuid.uuid4().hex[:6]}"
        profile = ImageProfile("eng", cache)
        assert profile.cache() == cache, "cache directory was not returned"


class TestDeckNamingReturnsConfiguredValues:
    """DeckNaming returns the configured name, prefix, and default values"""

    def test_returns_deck_name(self):
        name = f"Décк_{uuid.uuid4().hex[:6]}"
        naming = DeckNaming(name, "cards", "vocabulary.json")
        assert naming.name() == name, "deck name was not returned"

    def test_returns_output_prefix(self):
        prefix = f"pfx_{uuid.uuid4().hex[:6]}"
        naming = DeckNaming("Deck", prefix, "vocabulary.json")
        assert naming.prefix() == prefix, "output prefix was not returned"

    def test_returns_default_filename(self):
        default = f"vocab_{uuid.uuid4().hex[:6]}.json"
        naming = DeckNaming("Deck", "cards", default)
        assert naming.default() == default, "default filename was not returned"


class TestLanguageReturnsComposedProfiles:
    """Language returns the composed audio, imagery, naming, and mapping"""

    def test_returns_audio_profile(self):
        audio = AudioProfile(f"p_{uuid.uuid4().hex[:4]}.txt", "audio")
        imagery = ImageProfile("eng", "manga")
        naming = DeckNaming("Deck", "cards", "vocab.json")
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        lang = Language(audio, imagery, naming, mapping)
        assert lang.audio() is audio, "audio profile was not returned"

    def test_returns_imagery_profile(self):
        audio = AudioProfile("p.txt", "audio")
        imagery = ImageProfile(f"ocr_{uuid.uuid4().hex[:4]}", "manga")
        naming = DeckNaming("Deck", "cards", "vocab.json")
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        lang = Language(audio, imagery, naming, mapping)
        assert lang.imagery() is imagery, "imagery profile was not returned"

    def test_returns_naming_configuration(self):
        audio = AudioProfile("p.txt", "audio")
        imagery = ImageProfile("eng", "manga")
        naming = DeckNaming(f"N_{uuid.uuid4().hex[:4]}", "cards", "vocab.json")
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        lang = Language(audio, imagery, naming, mapping)
        assert lang.naming() is naming, "naming configuration was not returned"

    def test_returns_vocabulary_mapping(self):
        audio = AudioProfile("p.txt", "audio")
        imagery = ImageProfile("eng", "manga")
        naming = DeckNaming("Deck", "cards", "vocab.json")
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        lang = Language(audio, imagery, naming, mapping)
        assert lang.mapping() is mapping, "vocabulary mapping was not returned"
