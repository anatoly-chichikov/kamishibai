#!/usr/bin/env python3
"""
Unit tests for TextDetector language resolution
"""

import uuid
from unittest.mock import patch

import pytest

from manga import TextDetector


class TestTextDetectorResolvesDefaultLanguage:
    """
    TextDetector with default lang resolves to eng
    """

    def test_resolves_eng_when_no_lang_specified(self):
        languages = ["eng", "osd", f"fake_{uuid.uuid4().hex[:4]}"]
        with patch("manga.pytesseract.get_languages", return_value=languages):
            detector = TextDetector(50)
        assert detector._lang == "eng", "default lang did not resolve to eng"


class TestTextDetectorResolvesMultipleLanguages:
    """
    TextDetector resolves all requested languages when available
    """

    def test_resolves_both_languages_when_installed(self):
        tag = uuid.uuid4().hex[:4]
        languages = ["eng", "ell", f"snum_{tag}", "osd"]
        with patch("manga.pytesseract.get_languages", return_value=languages):
            detector = TextDetector(50, "eng+ell")
        assert detector._lang == "eng+ell", "both languages were not resolved"


class TestTextDetectorFallsBackOnMissing:
    """
    TextDetector drops unavailable languages and falls back gracefully
    """

    def test_keeps_only_available_languages(self):
        tag = uuid.uuid4().hex[:4]
        languages = ["eng", f"other_{tag}"]
        with patch("manga.pytesseract.get_languages", return_value=languages):
            detector = TextDetector(50, "eng+ell")
        assert detector._lang == "eng", "unavailable language was not dropped"

    def test_falls_back_to_eng_when_none_available(self):
        tag = uuid.uuid4().hex[:4]
        languages = [f"zyx_{tag}", "osd"]
        with patch("manga.pytesseract.get_languages", return_value=languages):
            detector = TextDetector(50, f"missing_{uuid.uuid4().hex[:4]}")
        assert detector._lang == "eng", "did not fall back to eng when no languages available"
