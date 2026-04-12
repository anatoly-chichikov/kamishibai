#!/usr/bin/env python3
"""Unit tests for target configuration classes."""

import uuid

from kamishibai.target import AudioProfile
from kamishibai.target import DeckNaming
from kamishibai.target import FontProfile
from kamishibai.target import ImageProfile
from kamishibai.target import LanguageProfile
from kamishibai.target import UiLabels


class TestAudioProfileReturnsConfiguredValues:
    """AudioProfile returns the configured language and cache values"""

    def test_returns_language_name(self):
        language = f"Language_{uuid.uuid4().hex[:6]}"
        profile = AudioProfile(language, "audio")
        assert profile.language() == language, "language name was not returned"

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


class TestFontProfileReturnsConfiguredValues:
    """FontProfile returns the configured report font family"""

    def test_returns_report_family(self):
        family = f"Font_{uuid.uuid4().hex[:6]}"
        profile = FontProfile(family)
        assert profile.report() == family, "report font family was not returned"


class TestUiLabelsReturnsConfiguredValues:
    """UiLabels returns the configured user-facing labels"""

    def test_returns_sentence_label(self):
        value = f"Sentence_{uuid.uuid4().hex[:6]}"
        labels = UiLabels(value, "Context", "Hint", "Importance")
        assert labels.sentence() == value, "sentence label was not returned"

    def test_returns_context_label(self):
        value = f"Context_{uuid.uuid4().hex[:6]}"
        labels = UiLabels("Sentence", value, "Hint", "Importance")
        assert labels.context() == value, "context label was not returned"

    def test_returns_hint_label(self):
        value = f"Hint_{uuid.uuid4().hex[:6]}"
        labels = UiLabels("Sentence", "Context", value, "Importance")
        assert labels.hint() == value, "hint label was not returned"

    def test_returns_importance_label(self):
        value = f"Importance_{uuid.uuid4().hex[:6]}"
        labels = UiLabels("Sentence", "Context", "Hint", value)
        assert labels.importance() == value, "importance label was not returned"


class TestLanguageProfileReturnsComposedProfiles:
    """LanguageProfile returns the composed audio, imagery, naming, and UI"""

    def test_returns_audio_profile(self):
        audio = AudioProfile(f"p_{uuid.uuid4().hex[:4]}.txt", "audio")
        imagery = ImageProfile("eng", "manga")
        naming = DeckNaming("Deck", "cards", "vocab.json")
        font = FontProfile("DejaVu Sans")
        labels = UiLabels("Translation", "Context", "Hint", "Importance")
        profile = LanguageProfile("en", audio, imagery, naming, font, labels)
        assert profile.audio() is audio, "audio profile was not returned"

    def test_returns_imagery_profile(self):
        audio = AudioProfile("p.txt", "audio")
        imagery = ImageProfile(f"ocr_{uuid.uuid4().hex[:4]}", "manga")
        naming = DeckNaming("Deck", "cards", "vocab.json")
        font = FontProfile("DejaVu Sans")
        labels = UiLabels("Translation", "Context", "Hint", "Importance")
        profile = LanguageProfile("en", audio, imagery, naming, font, labels)
        assert profile.imagery() is imagery, "imagery profile was not returned"

    def test_returns_naming_configuration(self):
        audio = AudioProfile("p.txt", "audio")
        imagery = ImageProfile("eng", "manga")
        naming = DeckNaming(f"N_{uuid.uuid4().hex[:4]}", "cards", "vocab.json")
        font = FontProfile("DejaVu Sans")
        labels = UiLabels("Translation", "Context", "Hint", "Importance")
        profile = LanguageProfile("en", audio, imagery, naming, font, labels)
        assert profile.naming() is naming, "naming configuration was not returned"

    def test_returns_target_code(self):
        audio = AudioProfile("p.txt", "audio")
        imagery = ImageProfile("eng", "manga")
        naming = DeckNaming("Deck", "cards", "vocab.json")
        font = FontProfile("DejaVu Sans")
        labels = UiLabels("Translation", "Context", "Hint", "Importance")
        profile = LanguageProfile("el", audio, imagery, naming, font, labels)
        assert profile.code() == "el", "language code was not returned"

    def test_returns_font_profile(self):
        audio = AudioProfile("p.txt", "audio")
        imagery = ImageProfile("eng", "manga")
        naming = DeckNaming("Deck", "cards", "vocab.json")
        font = FontProfile(f"Font_{uuid.uuid4().hex[:4]}")
        labels = UiLabels("Translation", "Context", "Hint", "Importance")
        profile = LanguageProfile("el", audio, imagery, naming, font, labels)
        assert profile.font() is font, "font profile was not returned"

    def test_returns_ui_labels(self):
        audio = AudioProfile("p.txt", "audio")
        imagery = ImageProfile("eng", "manga")
        naming = DeckNaming("Deck", "cards", "vocab.json")
        font = FontProfile("DejaVu Sans")
        labels = UiLabels(f"Sentence_{uuid.uuid4().hex[:4]}", "Context", "Hint", "Importance")
        profile = LanguageProfile("el", audio, imagery, naming, font, labels)
        assert profile.labels() is labels, "UI labels were not returned"
