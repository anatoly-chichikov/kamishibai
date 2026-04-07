#!/usr/bin/env python3
"""Unit tests for Vocabulary class."""

import json
import os
import tempfile
import uuid

import pytest

from kamishibai.deck import Vocabulary
from kamishibai.deck import VocabularyMapping


class TestVocabularyReadsValidEntry:
    """Vocabulary correctly parses valid source and target entries."""

    def test_reads_term_from_json_entry(self):
        term = f"testword_{uuid.uuid4().hex[:8]}"
        data = self._document([self._entry(term=term)])
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["word"] == term, "term field was not parsed correctly"

    def test_reads_pronunciation_from_json_entry(self):
        pronunciation = f"/ˈtɛst_{uuid.uuid4().hex[:4]}/"
        data = self._document([self._entry(pronunciation=pronunciation)])
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["pronunciation"] == pronunciation, "pronunciation was not parsed"

    def test_reads_meaning_from_json_entry(self):
        meaning = f"перевод_{uuid.uuid4().hex[:6]}"
        data = self._document([self._entry(meaning=meaning)])
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["translation"] == meaning, "meaning was not parsed"

    def test_reads_target_sentence_from_json_entry(self):
        sentence = f"Example with comma, and quotes '{uuid.uuid4().hex[:4]}'"
        data = self._document([self._entry(target=sentence)])
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["example"] == sentence, "target sentence was not parsed"

    def test_reads_source_sentence_from_json_entry(self):
        sentence = f"Русское предложение с запятой, и «кавычками» {uuid.uuid4().hex[:4]}"
        data = self._document([self._entry(source=sentence)])
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["sentence"] == sentence, "source sentence was not parsed"

    def test_reads_context_from_json_entry(self):
        context = f"Контекст использования, формальный стиль {uuid.uuid4().hex[:4]}"
        data = self._document([self._entry(context=context)])
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["context"] == context, "context was not parsed"

    def test_reads_importance_as_string_from_json_entry(self):
        importance = 8
        data = self._document([self._entry(importance=importance)])
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["importance"] == str(importance), "importance was not converted to string"

    def _document(self, entries):
        return {"entries": entries}

    def _entry(self, term="test", source="Тест", target="Test", meaning="", pronunciation="", transcription="", highlight="", hint="", context="", importance=""):
        return {
            "term": term,
            "meaning": meaning,
            "pronunciation": pronunciation,
            "transcription": transcription,
            "source": {
                "sentence": source,
                "highlight": highlight,
                "hint": hint,
                "context": context,
            },
            "target": {
                "sentence": target,
                "lang": "en",
            },
            "importance": importance,
        }

    def _write(self, data):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as file:
            json.dump(data, file, ensure_ascii=False)
        return path


class TestVocabularyFiltersInvalidEntries:
    """Vocabulary skips entries without required nested fields."""

    def test_skips_entry_without_term(self):
        data = self._document([{"source": {"sentence": "Предложение"}, "target": {"sentence": "Sentence", "lang": "en"}}])
        with pytest.raises(ValueError):
            Vocabulary(self._write(data), VocabularyMapping()).entries()

    def test_skips_entry_without_source_sentence(self):
        data = self._document([{"term": "test", "source": {}, "target": {"sentence": "Sentence", "lang": "en"}}])
        with pytest.raises(ValueError):
            Vocabulary(self._write(data), VocabularyMapping()).entries()

    def test_skips_entry_without_target_sentence(self):
        data = self._document([{"term": "test", "source": {"sentence": "Предложение"}, "target": {"lang": "en"}}])
        with pytest.raises(ValueError):
            Vocabulary(self._write(data), VocabularyMapping()).entries()

    def test_skips_entry_without_target_language(self):
        data = self._document([{"term": "test", "source": {"sentence": "Предложение"}, "target": {"sentence": "Sentence"}}])
        with pytest.raises(ValueError):
            Vocabulary(self._write(data), VocabularyMapping()).entries()

    def test_reads_only_valid_entries_from_mixed_data(self):
        valid = {
            "term": f"valid_{uuid.uuid4().hex[:4]}",
            "source": {"sentence": "Валидное"},
            "target": {"sentence": "Valid", "lang": "en"},
        }
        invalid = {"term": "invalid", "source": {"sentence": "Есть источник"}, "target": {"lang": "en"}}
        data = self._document([invalid, valid, {"source": {"sentence": "Без term"}, "target": {"sentence": "No term", "lang": "en"}}])
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert len(entries) == 1, "only one valid entry should be returned"

    def _document(self, entries):
        return {"entries": entries}

    def _write(self, data):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as file:
            json.dump(data, file, ensure_ascii=False)
        return path


class TestVocabularyCoalescesNullValues:
    """Vocabulary coalesces JSON null values to empty strings."""

    def test_null_pronunciation_becomes_empty_string(self):
        data = self._document([self._entry(pronunciation=None)])
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["pronunciation"] == "", "null pronunciation was not coalesced to empty string"

    def test_null_context_becomes_empty_string(self):
        data = self._document([self._entry(context=None)])
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["context"] == "", "null context was not coalesced to empty string"

    def test_null_importance_becomes_empty_string(self):
        data = self._document([self._entry(importance=None)])
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["importance"] == "", "null importance was not coalesced to empty string"

    def _document(self, entries):
        return {"entries": entries}

    def _entry(self, pronunciation="", context="", importance=""):
        return {
            "term": "test",
            "pronunciation": pronunciation,
            "source": {
                "sentence": "Предложение",
                "context": context,
            },
            "target": {
                "sentence": "Sentence",
                "lang": "en",
            },
            "importance": importance,
        }

    def _write(self, data):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as file:
            json.dump(data, file, ensure_ascii=False)
        return path


class TestVocabularyHandlesSpecialCharacters:
    """Vocabulary handles special characters in nested text fields."""

    def test_handles_commas_in_target_sentence(self):
        sentence = "The king said, 'I will punish you,' and left."
        data = self._document([self._entry(target=sentence)])
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["example"] == sentence, "commas in target sentence broke parsing"

    def test_handles_quotes_in_target_sentence(self):
        sentence = 'She whispered: "Don\'t go there."'
        data = self._document([self._entry(target=sentence)])
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["example"] == sentence, "quotes in target sentence broke parsing"

    def test_handles_unicode_characters_in_source_sentence(self):
        sentence = "Он сказал: «Привет» — и ушёл 🎭"
        data = self._document([self._entry(source=sentence)])
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["sentence"] == sentence, "unicode characters broke parsing"

    def _document(self, entries):
        return {"entries": entries}

    def _entry(self, source="Источник", target="Target"):
        return {
            "term": "test",
            "source": {"sentence": source},
            "target": {"sentence": target, "lang": "en"},
        }

    def _write(self, data):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as file:
            json.dump(data, file, ensure_ascii=False)
        return path


class TestVocabularyRejectsInvalidRoot:
    """Vocabulary raises ValueError when JSON root is not a valid document object."""

    def test_rejects_json_array_instead_of_object(self):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as file:
            json.dump([{"term": "слово"}], file, ensure_ascii=False)
        with pytest.raises(ValueError):
            Vocabulary(path, VocabularyMapping()).entries()

    def test_rejects_json_string_instead_of_object(self):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as file:
            json.dump(f"строка_{uuid.uuid4().hex[:4]}", file, ensure_ascii=False)
        with pytest.raises(ValueError):
            Vocabulary(path, VocabularyMapping()).entries()

    def test_rejects_missing_entries_array(self):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as file:
            json.dump({}, file, ensure_ascii=False)
        with pytest.raises(ValueError):
            Vocabulary(path, VocabularyMapping()).entries()


class TestVocabularyRejectsEmptyData:
    """Vocabulary raises ValueError when no valid entries remain after filtering."""

    def test_rejects_empty_entries_array(self):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as file:
            json.dump({"entries": []}, file)
        with pytest.raises(ValueError):
            Vocabulary(path, VocabularyMapping()).entries()

    def test_rejects_when_all_entries_lack_required_fields(self):
        data = {
            "entries": [
                {"source": {"sentence": f"Без term {uuid.uuid4().hex[:4]}"}},
                {"term": f"без_предложения_{uuid.uuid4().hex[:4]}"},
            ],
        }
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as file:
            json.dump(data, file, ensure_ascii=False)
        with pytest.raises(ValueError):
            Vocabulary(path, VocabularyMapping()).entries()


class TestVocabularyReadsTargetMetadata:
    """VocabularyMapping reads language-agnostic target fields."""

    def test_reads_target_sentence_for_greek_entry(self):
        example = f"Η γάτα κάθεται στο τραπέζι {uuid.uuid4().hex[:4]}"
        data = {
            "entries": [
                {
                    "term": "γάτα",
                    "source": {"sentence": "Кошка"},
                    "target": {"sentence": example, "lang": "el"},
                }
            ],
        }
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["example"] == example, "Greek target sentence was not read from target.sentence"

    def test_reads_transcription_from_transcription_field(self):
        transcription = f"i ɣata kaθete sto trapezi {uuid.uuid4().hex[:4]}"
        data = {
            "entries": [
                {
                    "term": "γάτα",
                    "transcription": transcription,
                    "source": {"sentence": "Кошка"},
                    "target": {"sentence": "Η γάτα κάθεται στο τραπέζι", "lang": "el"},
                }
            ],
        }
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["transcription"] == transcription, "transcription was not read from transcription field"

    def test_returns_empty_transcription_when_missing(self):
        data = {
            "entries": [
                {
                    "term": "test",
                    "source": {"sentence": "Тест"},
                    "target": {"sentence": "Test", "lang": "en"},
                }
            ],
        }
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["transcription"] == "", "missing transcription did not become empty string"

    def test_reads_target_language_from_target_metadata(self):
        data = {
            "entries": [
                {
                    "term": "γάτα",
                    "source": {"sentence": "Кошка"},
                    "target": {"sentence": "Η γάτα κάθεται στο τραπέζι", "lang": "el"},
                }
            ],
        }
        entries = Vocabulary(self._write(data), VocabularyMapping()).entries()
        assert entries[0]["target_lang"] == "el", "target language was not read from target metadata"

    def _write(self, data):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as file:
            json.dump(data, file, ensure_ascii=False)
        return path
