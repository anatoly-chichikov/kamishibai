#!/usr/bin/env python3
"""
Unit tests for TextDetector language resolution
"""

import uuid
from unittest.mock import patch

import pytest

from kamishibai.scene import TextDetector
from kamishibai.scene import TextDetectors


class TestTextDetectorResolvesDefaultLanguage:
    """
    TextDetector with default lang resolves to eng
    """

    def test_resolves_eng_when_no_lang_specified(self):
        languages = ["eng", "osd", f"fake_{uuid.uuid4().hex[:4]}"]
        with patch("kamishibai.scene.pytesseract.get_languages", return_value=languages):
            detector = TextDetector(50)
        assert detector._lang == "eng", "default lang did not resolve to eng"


class TestTextDetectorResolvesMultipleLanguages:
    """
    TextDetector resolves all requested languages when available
    """

    def test_resolves_both_languages_when_installed(self):
        tag = uuid.uuid4().hex[:4]
        languages = ["eng", "ell", f"snum_{tag}", "osd"]
        with patch("kamishibai.scene.pytesseract.get_languages", return_value=languages):
            detector = TextDetector(50, "eng+ell")
        assert detector._lang == "eng+ell", "both languages were not resolved"


class TestTextDetectorFallsBackOnMissing:
    """
    TextDetector drops unavailable languages and falls back gracefully
    """

    def test_keeps_only_available_languages(self):
        tag = uuid.uuid4().hex[:4]
        languages = ["eng", f"other_{tag}"]
        with patch("kamishibai.scene.pytesseract.get_languages", return_value=languages):
            detector = TextDetector(50, "eng+ell")
        assert detector._lang == "eng", "unavailable language was not dropped"

    def test_falls_back_to_eng_when_none_available(self):
        tag = uuid.uuid4().hex[:4]
        languages = [f"zyx_{tag}", "osd"]
        with patch("kamishibai.scene.pytesseract.get_languages", return_value=languages):
            detector = TextDetector(50, f"missing_{uuid.uuid4().hex[:4]}")
        assert detector._lang == "eng", "did not fall back to eng when no languages available"


class _Detector:
    """Records image lookups and returns a fixed value."""

    def __init__(self, value):
        self._value = value
        self._calls = 0

    def detected(self, image):
        """Return the fixed value and count invocations."""
        self._calls += 1
        return self._value


class TestTextDetectorsSelectsByTargetLanguage:
    """TextDetectors routes OCR calls by scene target language."""

    def test_uses_target_specific_detector(self):
        english = _Detector("english")
        greek = _Detector("greek")
        detectors = TextDetectors({"en": english, "el": greek}, _Detector("fallback"))
        scene = {"manga_panel": {"meta": {"target_lang": "el"}}}
        result = detectors.detected(scene, object())
        assert result == "greek", "target-specific detector was not selected"

    def test_uses_fallback_for_unknown_target_language(self):
        fallback = _Detector("fallback")
        detectors = TextDetectors({"en": _Detector("english")}, fallback)
        scene = {"manga_panel": {"meta": {"target_lang": f"x_{uuid.uuid4().hex[:4]}"}}}
        result = detectors.detected(scene, object())
        assert result == "fallback", "fallback detector was not selected for unknown target language"
