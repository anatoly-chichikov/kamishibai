#!/usr/bin/env python3
"""
Unit tests for Pipeline class
"""

import uuid

import pytest

from deck import Pipeline


class _FailingAudio:
    """Fake audio generator that raises a configurable exception"""

    def __init__(self, error):
        self._error = error
        self._cache = _FakeCache()

    def generate(self, text):
        raise self._error


class _FailingIllustration:
    """Fake illustration generator that raises a configurable exception"""

    def __init__(self, error):
        self._error = error
        self._cache = _FakeCache()

    def generate(self, sentence, word):
        raise self._error


class _SuccessAudio:
    """Fake audio generator that always succeeds"""

    def __init__(self):
        self._cache = _FakeCache()

    def generate(self, text):
        return (f"{uuid.uuid4().hex[:12]}.wav", False)


class _SuccessIllustration:
    """Fake illustration generator that always succeeds"""

    def __init__(self):
        self._cache = _FakeCache()

    def generate(self, sentence, word):
        return (f"{uuid.uuid4().hex[:12]}.jpg", False)


class _FakeCache:
    """Fake cache that returns a temporary path"""

    def filepath(self, filename):
        return f"/tmp/{filename}"


class _FakeDeck:
    """Fake deck that records operations"""

    def __init__(self):
        self._notes = []
        self._attached = []

    def add(self, entry, audio, image):
        self._notes.append(entry["word"])

    def attach(self, filepath):
        self._attached.append(filepath)


class TestPipelineSurvivesServerError:
    """
    Pipeline continues processing remaining entries when a non-ValueError
    exception (such as ServerError) is raised during generation
    """

    def test_continues_after_audio_server_error(self):
        word = f"wörd_{uuid.uuid4().hex[:6]}"
        error = RuntimeError("503 UNAVAILABLE: сервер недоступен")
        audio = _FailingAudio(error)
        illustration = _SuccessIllustration()
        deck = _FakeDeck()
        pipeline = Pipeline(audio, illustration, deck)
        entries = [{"word": word, "example": "a séntence", "sentence": "предложение"}]
        failures = pipeline.process(entries)
        assert len(failures) == 1, "server error in audio did not record a failure"

    def test_continues_after_image_server_error(self):
        word = f"wörd_{uuid.uuid4().hex[:6]}"
        error = RuntimeError("503 UNAVAILABLE: сервер недоступен")
        audio = _SuccessAudio()
        illustration = _FailingIllustration(error)
        deck = _FakeDeck()
        pipeline = Pipeline(audio, illustration, deck)
        entries = [{"word": word, "example": "a séntence", "sentence": "предложение"}]
        failures = pipeline.process(entries)
        assert len(failures) == 1, "server error in image did not record a failure"

    def test_processes_remaining_entries_after_server_error(self):
        error = RuntimeError("503 UNAVAILABLE: сервер недоступен")
        failing = _FailingAudio(error)
        succeeding = _SuccessAudio()
        illustration = _SuccessIllustration()
        deck = _FakeDeck()
        word_ok = f"wörd_{uuid.uuid4().hex[:6]}"
        word_fail = f"fäil_{uuid.uuid4().hex[:6]}"
        entries = [
            {"word": word_fail, "example": "séntence one", "sentence": "предложение"},
            {"word": word_ok, "example": "séntence two", "sentence": "предложение"},
        ]
        pipeline_fail = Pipeline(failing, illustration, _FakeDeck())
        pipeline_ok = Pipeline(succeeding, illustration, deck)
        failures_first = pipeline_fail.process(entries[:1])
        failures_second = pipeline_ok.process(entries[1:])
        assert len(failures_first) == 1, "first entry failure was not recorded"
        assert len(failures_second) == 0, "second entry should have succeeded"
