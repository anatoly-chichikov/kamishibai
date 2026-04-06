#!/usr/bin/env python3
"""Unit tests for kamishibai application helpers."""

import uuid

from kamishibai import app


class TestLanguageSelection:
    """The unified CLI returns language-specific configuration by code."""

    def test_selects_default_english_profile(self):
        language = app._language("en")
        assert language.naming().name() == "English Vocabulary", \
            "English CLI configuration was not selected"

    def test_selects_greek_profile(self):
        language = app._language("el")
        assert language.naming().default() == "vocabulary_greek.json", \
            "Greek CLI configuration was not selected"


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
