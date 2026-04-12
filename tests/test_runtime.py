#!/usr/bin/env python3
"""Tests for prompt rendering helpers."""

import tempfile
import uuid

from kamishibai.runtime import Media
from kamishibai.runtime import audio_prompt
from kamishibai.runtime import scene_prompt
from kamishibai.target import AudioProfile
from kamishibai.target import DeckNaming
from kamishibai.target import FontProfile
from kamishibai.target import ImageProfile
from kamishibai.target import LanguageProfile
from kamishibai.target import UiLabels


class TestAudioPromptUsesSharedTemplate:
    """Audio prompt rendering comes from the shared asset template."""

    def test_renders_language_into_audio_prompt(self):
        language = f"Language_{uuid.uuid4().hex[:4]}"
        result = audio_prompt(language)
        assert result == f"Say in natural {language}: {{text}}", "shared audio prompt was not rendered from the common template"


class TestScenePromptUsesSharedTemplate:
    """Scene prompt rendering comes from the shared asset template."""

    def test_renders_language_into_scene_prompt(self):
        language = f"Language_{uuid.uuid4().hex[:4]}"
        result = scene_prompt(language)
        assert f"educational {language} flashcards" in result, "shared scene prompt did not include the target language"

    def test_preserves_json_schema_braces_for_second_format_pass(self):
        language = f"Language_{uuid.uuid4().hex[:4]}"
        result = scene_prompt(language).format(sentence="demo")
        assert '"x": int' in result, "scene prompt lost JSON schema braces during formatting"


class _Profiles:
    """Fake profile registry for runtime wiring tests."""

    def __init__(self, item, fallback):
        self._item = item
        self._fallback = fallback

    def item(self, code):
        """Return the configured language profile for the requested code."""
        if code == self._item.code():
            return self._item
        raise ValueError(f"Unsupported target language '{code}'")

    def fallback_ocr(self):
        """Return the configured OCR fallback string."""
        return self._fallback


class TestMediaUsesInjectedProfiles:
    """Media wiring reads all runtime language settings from injected profiles."""

    def test_supports_new_language_from_profile_registry(self, monkeypatch):
        monkeypatch.setattr("kamishibai.scene.pytesseract.get_languages", lambda: ["eng", "gle", "osd"])
        item = LanguageProfile(
            "ga",
            AudioProfile("Irish", "audio-ga"),
            ImageProfile("eng+gle", "manga-ga"),
            DeckNaming("Irish Vocabulary", "ga", "kamishibai.json"),
            FontProfile("DejaVu Sans"),
            UiLabels("Translation", "Context", "Hint", "Importance"),
        )
        media = Media(object(), tempfile.mkdtemp(), _Profiles(item, "osd"))
        audio = media.audio({"target_lang": "ga"})
        media.illustration({"target_lang": "ga"})
        assert audio._cache.path().endswith("audio-ga"), "audio cache was not derived from the injected profile"

    def test_uses_injected_fallback_ocr_policy(self, monkeypatch):
        monkeypatch.setattr("kamishibai.scene.pytesseract.get_languages", lambda: ["eng", "gle", "osd"])
        item = LanguageProfile(
            "ga",
            AudioProfile("Irish", "audio-ga"),
            ImageProfile("eng+gle", "manga-ga"),
            DeckNaming("Irish Vocabulary", "ga", "kamishibai.json"),
            FontProfile("DejaVu Sans"),
            UiLabels("Translation", "Context", "Hint", "Importance"),
        )
        media = Media(object(), tempfile.mkdtemp(), _Profiles(item, "osd"))
        assert media._renderer._text._fallback._lang == "osd", "fallback OCR was not derived from the injected profiles"
