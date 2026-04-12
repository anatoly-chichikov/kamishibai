#!/usr/bin/env python3
"""Unit tests for kamishibai application helpers."""

import uuid

from kamishibai import cli
from kamishibai.config import Fonts
from kamishibai.config import Labels
from kamishibai.config import naming
from kamishibai.config import profile


class TestProfileSelection:
    """The unified CLI returns target profiles by code."""

    def test_selects_english_profile(self):
        item = profile("en")
        assert item.naming().name() == "English Vocabulary", \
            "English target profile was not selected"

    def test_selects_greek_profile(self):
        item = profile("el")
        assert item.naming().default() == "kamishibai.json", \
            "Greek target profile was not selected"

    def test_selects_spanish_profile(self):
        item = profile("es")
        assert item.audio().language() == "Spanish", \
            "Spanish target profile was not selected"

    def test_selects_german_profile(self):
        item = profile("de")
        assert item.imagery().ocr() == "eng+deu", \
            "German target profile was not selected"

    def test_selects_chinese_profile(self):
        item = profile("zh")
        assert item.imagery().ocr() == "eng+chi_sim", \
            "Chinese target profile was not selected"

    def test_selects_russian_profile(self):
        item = profile("ru")
        assert item.labels().sentence() == "Перевод", \
            "Russian source profile was not selected"

    def test_rejects_unknown_profile(self):
        try:
            profile(f"x_{uuid.uuid4().hex[:4]}")
        except ValueError:
            pass
        else:
            assert False, "unknown target profile was not rejected"


class TestNaming:
    """The application derives deck naming from CLI input and schema."""

    def test_uses_target_profile_name_by_default(self):
        entries = [{"source_lang": "ru", "target_lang": "el"}]
        item = naming(cli.arguments(["file.json"]), entries)
        assert item.name() == "Greek Vocabulary", \
            "default deck name was not derived from target profile"

    def test_uses_custom_name_when_provided(self):
        value = f"test deck {uuid.uuid4().hex[:4]}"
        entries = [{"source_lang": "ru", "target_lang": "el"}]
        item = naming(cli.arguments(["--deck", value, "file.json"]), entries)
        assert item.name() == value, \
            "custom deck name was not used"

    def test_derives_prefix_from_name(self):
        value = f"test demo {uuid.uuid4().hex[:4]}"
        entries = [{"source_lang": "ru", "target_lang": "el"}]
        item = naming(cli.arguments(["--deck", value, "file.json"]), entries)
        assert item.prefix().startswith("test-demo"), \
            "deck prefix was not derived from the deck name"

    def test_uses_generic_name_for_mixed_targets(self):
        entries = [
            {"source_lang": "ru", "target_lang": "el"},
            {"source_lang": "ru", "target_lang": "zh"},
        ]
        item = naming(cli.arguments(["file.json"]), entries)
        assert item.name() == "Kamishibai Deck", \
            "mixed targets did not fall back to generic deck name"


class TestFontSelection:
    """The application selects a report font based on each entry target."""

    def test_uses_default_font_when_languages_are_missing(self):
        font = Fonts().selected({})
        assert font._regular._family == "DejaVu Sans", \
            "default report font was not selected when entry languages were missing"

    def test_uses_default_font_without_chinese_target(self):
        font = Fonts().selected({"source_lang": "ru", "target_lang": "es"})
        assert font._regular._family == "DejaVu Sans", \
            "default report font was not selected for non-Chinese target"

    def test_uses_cjk_font_for_chinese_target(self):
        font = Fonts().selected({"source_lang": "ru", "target_lang": "zh"})
        assert font._regular._family == "Hiragino Sans GB", \
            "CJK report font was not selected for Chinese target"

    def test_uses_cjk_font_for_chinese_source(self):
        font = Fonts().selected({"source_lang": "zh", "target_lang": "en"})
        assert font._regular._family == "Hiragino Sans GB", \
            "CJK report font was not selected for Chinese source"


class TestLabelSelection:
    """The application selects report labels from source language profiles."""

    def test_uses_russian_labels_for_russian_source(self):
        labels = Labels().selected({"source_lang": "ru"})
        assert labels.sentence() == "Перевод", \
            "Russian source labels were not selected"

    def test_uses_default_labels_when_source_is_missing(self):
        labels = Labels().selected({})
        assert labels.sentence() == "Translation", \
            "default labels were not selected when source language was missing"


class _Diagnosis:
    """Records diagnosis output for run error handling tests."""

    def __init__(self, items):
        """Store a shared recording list."""
        self._items = items

    def show(self, message, path):
        """Record the reported message and path."""
        self._items.append((message, path))


class _Selector:
    """Returns a recording diagnosis object."""

    def __init__(self, items):
        """Store a shared recording list."""
        self._items = items

    def selected(self):
        """Return a diagnosis recorder."""
        return _Diagnosis(self._items)


class TestRun:
    """The public run helper maps application outcomes to exit codes."""

    def test_returns_zero_when_main_succeeds(self, monkeypatch):
        value = ["--deck", f"Deck_{uuid.uuid4().hex[:4]}"]
        monkeypatch.setattr(cli, "main", lambda argv: None)
        assert cli.run(value) == 0, \
            "run did not return zero after successful execution"

    def test_returns_130_when_main_is_interrupted(self, monkeypatch):
        def boom(argv):
            """Raise KeyboardInterrupt for the test."""
            raise KeyboardInterrupt
        monkeypatch.setattr(cli, "main", boom)
        assert cli.run([f"λέξη_{uuid.uuid4().hex[:4]}.json"]) == 130, \
            "run did not translate KeyboardInterrupt to exit code 130"

    def test_returns_one_when_main_fails(self, monkeypatch):
        items = []

        def boom(argv):
            """Raise ValueError for the test."""
            raise ValueError("problem")

        monkeypatch.setattr(cli, "main", boom)
        monkeypatch.setattr(cli, "DiagnosisSelector", lambda terminal: _Selector(items))
        assert cli.run([f"λέξη_{uuid.uuid4().hex[:4]}.json"]) == 1, \
            "run did not translate ValueError to exit code 1"

    def test_reports_failures_to_diagnosis(self, monkeypatch):
        items = []

        def boom(argv):
            """Raise FileNotFoundError for the test."""
            raise FileNotFoundError(2, "missing", f"/tmp/{uuid.uuid4().hex[:4]}.json")

        monkeypatch.setattr(cli, "main", boom)
        monkeypatch.setattr(cli, "DiagnosisSelector", lambda terminal: _Selector(items))
        cli.run([f"λέξη_{uuid.uuid4().hex[:4]}.json"])
        assert len(items) == 1, \
            "run did not report the failure through diagnosis"
