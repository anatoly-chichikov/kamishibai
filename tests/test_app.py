#!/usr/bin/env python3
"""Unit tests for kamishibai application helpers."""

import uuid

from kamishibai import app


class TestProfileSelection:
    """The unified CLI returns target profiles by code."""

    def test_selects_english_profile(self):
        profile = app._profile("en")
        assert profile.naming().name() == "English Vocabulary", \
            "English target profile was not selected"

    def test_selects_greek_profile(self):
        profile = app._profile("el")
        assert profile.naming().default() == "kamishibai.json", \
            "Greek target profile was not selected"

    def test_selects_spanish_profile(self):
        profile = app._profile("es")
        assert profile.audio().prompt() == "Say in natural Spanish: {text}", \
            "Spanish target profile was not selected"

    def test_selects_german_profile(self):
        profile = app._profile("de")
        assert profile.imagery().ocr() == "eng+deu", \
            "German target profile was not selected"

    def test_selects_chinese_profile(self):
        profile = app._profile("zh")
        assert profile.imagery().ocr() == "eng+chi_sim", \
            "Chinese target profile was not selected"

    def test_rejects_unknown_profile(self):
        try:
            app._profile(f"x_{uuid.uuid4().hex[:4]}")
        except ValueError:
            pass
        else:
            assert False, "unknown target profile was not rejected"


class TestNaming:
    """The application derives generic deck naming from CLI input."""

    def test_uses_generic_default_name(self):
        naming = app._naming(app._arguments(["file.json"]))
        assert naming.name() == "Kamishibai Deck", \
            "default deck name was not used"

    def test_uses_custom_name_when_provided(self):
        value = f"test deck {uuid.uuid4().hex[:4]}"
        naming = app._naming(app._arguments(["--deck", value, "file.json"]))
        assert naming.name() == value, \
            "custom deck name was not used"

    def test_derives_prefix_from_name(self):
        value = f"test demo {uuid.uuid4().hex[:4]}"
        naming = app._naming(app._arguments(["--deck", value, "file.json"]))
        assert naming.prefix().startswith("test-demo"), \
            "deck prefix was not derived from the deck name"


class TestFontSelection:
    """The application selects a report font based on each entry target."""

    def test_uses_default_font_when_target_is_missing(self):
        font = app._Fonts().selected({})
        assert font._regular._family == "DejaVu Sans", \
            "default report font was not selected when target language was missing"

    def test_uses_default_font_without_chinese_target(self):
        font = app._Fonts().selected({"target_lang": "es"})
        assert font._regular._family == "DejaVu Sans", \
            "default report font was not selected for non-Chinese target"

    def test_uses_cjk_font_for_chinese_target(self):
        font = app._Fonts().selected({"target_lang": "zh"})
        assert font._regular._family == "Hiragino Sans GB", \
            "CJK report font was not selected for Chinese target"


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
        monkeypatch.setattr(app, "main", lambda argv: None)
        assert app.run(value) == 0, \
            "run did not return zero after successful execution"

    def test_returns_130_when_main_is_interrupted(self, monkeypatch):
        def boom(argv):
            """Raise KeyboardInterrupt for the test."""
            raise KeyboardInterrupt
        monkeypatch.setattr(app, "main", boom)
        assert app.run([f"λέξη_{uuid.uuid4().hex[:4]}.json"]) == 130, \
            "run did not translate KeyboardInterrupt to exit code 130"

    def test_returns_one_when_main_fails(self, monkeypatch):
        items = []

        def boom(argv):
            """Raise ValueError for the test."""
            raise ValueError("problem")

        monkeypatch.setattr(app, "main", boom)
        monkeypatch.setattr(app, "DiagnosisSelector", lambda terminal: _Selector(items))
        assert app.run([f"λέξη_{uuid.uuid4().hex[:4]}.json"]) == 1, \
            "run did not translate ValueError to exit code 1"

    def test_reports_failures_to_diagnosis(self, monkeypatch):
        items = []

        def boom(argv):
            """Raise FileNotFoundError for the test."""
            raise FileNotFoundError(2, "missing", f"/tmp/{uuid.uuid4().hex[:4]}.json")

        monkeypatch.setattr(app, "main", boom)
        monkeypatch.setattr(app, "DiagnosisSelector", lambda terminal: _Selector(items))
        app.run([f"λέξη_{uuid.uuid4().hex[:4]}.json"])
        assert len(items) == 1, \
            "run did not report the failure through diagnosis"
