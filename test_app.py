#!/usr/bin/env python3
"""Unit tests for kamishibai CLI wrappers."""

import uuid

import create_anki_deck
import create_anki_deck_greek

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


class TestLegacyWrappers:
    """Legacy wrapper scripts delegate into the unified kamishibai CLI."""

    def test_default_wrapper_forwards_argv(self, monkeypatch):
        seen = []
        code = 17
        value = ["--deck", f"Deck_{uuid.uuid4().hex[:4]}"]

        def fake(argv):
            seen.append(argv)
            return code

        monkeypatch.setattr(create_anki_deck, "run_legacy_default", fake)
        create_anki_deck.main(value)
        assert seen == [value], \
            "default wrapper did not forward argv to the unified CLI"

    def test_greek_wrapper_forwards_argv(self, monkeypatch):
        seen = []
        code = 19

        def fake(argv):
            seen.append(argv)
            return code

        monkeypatch.setattr(create_anki_deck_greek, "run_legacy_greek", fake)
        value = [f"λέξη_{uuid.uuid4().hex[:4]}.json"]
        result = create_anki_deck_greek.main(value)
        assert seen == [value], \
            "Greek wrapper did not forward argv to the unified CLI"

    def test_default_wrapper_returns_delegate_exit_code(self, monkeypatch):
        monkeypatch.setattr(create_anki_deck, "run_legacy_default", lambda argv: 23)
        assert create_anki_deck.main([f"--deck=Deck_{uuid.uuid4().hex[:4]}"]) == 23, \
            "default wrapper did not return the delegate exit code"

    def test_greek_wrapper_returns_delegate_exit_code(self, monkeypatch):
        monkeypatch.setattr(create_anki_deck_greek, "run_legacy_greek", lambda argv: 29)
        assert create_anki_deck_greek.main([f"λέξη_{uuid.uuid4().hex[:4]}.json"]) == 29, \
            "Greek wrapper did not return the delegate exit code"
