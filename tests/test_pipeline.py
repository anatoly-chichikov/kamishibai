#!/usr/bin/env python3
"""
Unit tests for Pipeline class
"""

import uuid

import pytest

from kamishibai.deck import Pipeline
from kamishibai.deck import StableId
from kamishibai.deck import VocabularyDeck
from kamishibai.target import DeckNaming


class _FailingAudio:
    """Fake audio generator that raises a configurable exception"""

    def __init__(self, error):
        self._error = error

    def generate(self, text):
        """Always raise the configured error"""
        raise self._error

    def filepath(self, filename):
        """Return a temporary path"""
        return f"/tmp/{filename}"


class _FailingIllustration:
    """Fake illustration generator that raises a configurable exception"""

    def __init__(self, error):
        self._error = error

    def generate(self, sentence, word, target, progress):
        """Always raise the configured error"""
        raise self._error

    def filepath(self, filename):
        """Return a temporary path"""
        return f"/tmp/{filename}"


class _SuccessAudio:
    """Fake audio generator that always succeeds"""

    def generate(self, text):
        """Return a random wav filename"""
        return (f"{uuid.uuid4().hex[:12]}.wav", False)

    def filepath(self, filename):
        """Return a temporary path"""
        return f"/tmp/{filename}"


class _SuccessIllustration:
    """Fake illustration generator that always succeeds"""

    def generate(self, sentence, word, target, progress):
        """Return a random jpg filename"""
        return (f"{uuid.uuid4().hex[:12]}.jpg", False)

    def filepath(self, filename):
        """Return a temporary path"""
        return f"/tmp/{filename}"


class _FakeProgress:
    """Fake progress that silently records events"""

    def card(self, index, total, word):
        """No-op"""

    def step(self, name):
        """No-op"""

    def done(self, name, label, path=""):
        """No-op"""

    def retry(self, name, attempt, reason):
        """No-op"""

    def skip(self, word, reason):
        """No-op"""


class _FakeDeck:
    """Fake deck that records operations"""

    def __init__(self):
        self._notes = []
        self._attached = []

    def add(self, entry, audio, image):
        """Record the added note"""
        self._notes.append(entry["word"])

    def attach(self, filepath):
        """Record the attached file"""
        self._attached.append(filepath)


class _RecordingAudio:
    """Fake audio that records all texts it was asked to generate"""

    def __init__(self):
        self._texts = []

    def generate(self, text):
        """Record the text and raise if empty"""
        self._texts.append(text)
        if not text.strip():
            raise ValueError("Cannot generate audio for empty text")
        return (f"{uuid.uuid4().hex[:12]}.wav", False)

    def filepath(self, filename):
        """Return a temporary path"""
        return f"/tmp/{filename}"


class _FakeGenanki:
    """Fake genanki.Deck that does nothing"""

    def add_note(self, note):
        """No-op"""


class _FakeFormat:
    """Fake NoteFormat that returns None"""

    def note(self, entry, audio, image):
        """Return None as a placeholder note"""
        return None


class TestVocabularyDeckDeduplicatesMedia:
    """VocabularyDeck does not add the same media file path twice"""

    def test_duplicate_path_attached_once(self):
        """VocabularyDeck.attach skips a path already in the media list"""
        container = VocabularyDeck(_FakeGenanki(), _FakeFormat(), [])
        path = f"/tmp/{uuid.uuid4().hex[:8]}.wav"
        container.attach(path)
        container.attach(path)
        assert len(container._media) == 1, \
            "duplicate media file path was not deduplicated"

    def test_different_paths_both_attached(self):
        """VocabularyDeck.attach keeps distinct paths"""
        container = VocabularyDeck(_FakeGenanki(), _FakeFormat(), [])
        first = f"/tmp/{uuid.uuid4().hex[:8]}.wav"
        second = f"/tmp/{uuid.uuid4().hex[:8]}.jpg"
        container.attach(first)
        container.attach(second)
        assert len(container._media) == 2, \
            "distinct media paths were incorrectly deduplicated"


class TestPipelineSkipsEntryWithEmptyExample:
    """Pipeline gracefully skips entries with empty example text"""

    def test_empty_example_recorded_as_failure(self):
        """Pipeline records failure when example is empty string"""
        word = f"wörd_{uuid.uuid4().hex[:6]}"
        audio = _RecordingAudio()
        illustration = _SuccessIllustration()
        deck = _FakeDeck()
        progress = _FakeProgress()
        pipeline = Pipeline(audio, illustration, deck, progress)
        entries = [{"word": word, "example": "", "sentence": "\u043f\u0440\u0435\u0434\u043b\u043e\u0436\u0435\u043d\u0438\u0435", "target_lang": "en"}]
        failures, _ = pipeline.process(entries)
        assert len(failures) == 1, \
            "empty example did not produce a failure"


class TestPipelineSurvivesServerError:
    """
    Pipeline continues processing remaining entries when a non-ValueError
    exception (such as ServerError) is raised during generation
    """

    def test_continues_after_audio_server_error(self):
        """Pipeline records failure when audio generation raises a server error"""
        word = f"wörd_{uuid.uuid4().hex[:6]}"
        error = RuntimeError("503 UNAVAILABLE: сервер недоступен")
        audio = _FailingAudio(error)
        illustration = _SuccessIllustration()
        deck = _FakeDeck()
        progress = _FakeProgress()
        pipeline = Pipeline(audio, illustration, deck, progress)
        entries = [{"word": word, "example": "a séntence", "sentence": "предложение", "target_lang": "en"}]
        failures, _ = pipeline.process(entries)
        assert len(failures) == 1, "server error in audio did not record a failure"

    def test_continues_after_image_server_error(self):
        """Pipeline records failure when image generation raises a server error"""
        word = f"wörd_{uuid.uuid4().hex[:6]}"
        error = RuntimeError("503 UNAVAILABLE: сервер недоступен")
        audio = _SuccessAudio()
        illustration = _FailingIllustration(error)
        deck = _FakeDeck()
        progress = _FakeProgress()
        pipeline = Pipeline(audio, illustration, deck, progress)
        entries = [{"word": word, "example": "a séntence", "sentence": "предложение", "target_lang": "en"}]
        failures, _ = pipeline.process(entries)
        assert len(failures) == 1, "server error in image did not record a failure"

    def test_processes_remaining_entries_after_server_error(self):
        """Pipeline processes subsequent entries after one fails with a server error"""
        error = RuntimeError("503 UNAVAILABLE: сервер недоступен")
        failing = _FailingAudio(error)
        succeeding = _SuccessAudio()
        illustration = _SuccessIllustration()
        deck = _FakeDeck()
        word_ok = f"wörd_{uuid.uuid4().hex[:6]}"
        word_fail = f"fäil_{uuid.uuid4().hex[:6]}"
        entries = [
            {"word": word_fail, "example": "séntence one", "sentence": "предложение", "target_lang": "en"},
            {"word": word_ok, "example": "séntence two", "sentence": "предложение", "target_lang": "en"},
        ]
        progress = _FakeProgress()
        pipeline_fail = Pipeline(failing, illustration, _FakeDeck(), progress)
        pipeline_ok = Pipeline(succeeding, illustration, deck, progress)
        failures_first, _ = pipeline_fail.process(entries[:1])
        failures_second, _ = pipeline_ok.process(entries[1:])
        assert len(failures_first) == 1, "first entry failure was not recorded"
        assert len(failures_second) == 0, "second entry should have succeeded"


class TestDeckOverrideProducesDifferentIdentity:
    """Custom --deck name produces a different StableId than the language default"""

    def test_custom_name_changes_deck_id(self):
        """DeckNaming with custom name yields a different StableId value"""
        custom = f"Décк_{uuid.uuid4().hex[:6]}"
        original = DeckNaming("English Vocabulary", "cards", "vocabulary.json")
        overridden = DeckNaming(custom, original.prefix(), original.default())
        assert StableId(overridden.name()).value() != StableId(original.name()).value(), \
            "custom deck name did not change the StableId"

    def test_custom_name_changes_model_id(self):
        """DeckNaming with custom name yields a different model StableId value"""
        custom = f"Décк_{uuid.uuid4().hex[:6]}"
        original = DeckNaming("English Vocabulary", "cards", "vocabulary.json")
        overridden = DeckNaming(custom, original.prefix(), original.default())
        assert StableId(f"{overridden.name()} Model").value() != StableId(f"{original.name()} Model").value(), \
            "custom deck name did not change the model StableId"

    def test_override_preserves_prefix(self):
        """DeckNaming override keeps the original prefix intact"""
        prefix = f"pfx_{uuid.uuid4().hex[:6]}"
        original = DeckNaming("English Vocabulary", prefix, "vocabulary.json")
        overridden = DeckNaming("Cüstom Nàme", original.prefix(), original.default())
        assert overridden.prefix() == prefix, \
            "override did not preserve the original prefix"

    def test_override_preserves_default(self):
        """DeckNaming override keeps the original default filename intact"""
        default = f"vocab_{uuid.uuid4().hex[:6]}.json"
        original = DeckNaming("English Vocabulary", "cards", default)
        overridden = DeckNaming("Cüstom Nàme", original.prefix(), original.default())
        assert overridden.default() == default, \
            "override did not preserve the original default filename"
