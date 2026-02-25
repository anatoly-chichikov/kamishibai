#!/usr/bin/env python3
"""
Unit tests for Vocabulary class
"""

import json
import os
import tempfile
import uuid

import pytest

from deck import Vocabulary
from deck import VocabularyMapping


class TestVocabularyReadsValidEntry:
    """
    Vocabulary correctly parses valid vocabulary entries
    """

    def test_reads_word_from_json_entry(self):
        word = f"testword_{uuid.uuid4().hex[:8]}"
        data = [{"word": word, "sentence_ru": "Тестовое предложение"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["word"] == word, "word field was not parsed correctly"

    def test_reads_pronunciation_from_json_entry(self):
        pronunciation = f"/ˈtɛst_{uuid.uuid4().hex[:4]}/"
        data = [{"word": "test", "pronunciation": pronunciation, "sentence_ru": "Тест"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["pronunciation"] == pronunciation, "pronunciation was not parsed"

    def test_reads_translation_from_json_entry(self):
        translation = f"перевод_{uuid.uuid4().hex[:6]}"
        data = [{"word": "test", "translation_ru": translation, "sentence_ru": "Тест"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["translation"] == translation, "translation was not parsed"

    def test_reads_example_sentence_from_json_entry(self):
        example = f"Example with comma, and quotes '{uuid.uuid4().hex[:4]}'"
        data = [{"word": "test", "sentence_en": example, "sentence_ru": "Тест"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["example"] == example, "example sentence was not parsed"

    def test_reads_russian_sentence_from_json_entry(self):
        sentence = f"Русское предложение с запятой, и «кавычками» {uuid.uuid4().hex[:4]}"
        data = [{"word": "test", "sentence_ru": sentence}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["sentence"] == sentence, "Russian sentence was not parsed"

    def test_reads_context_from_json_entry(self):
        context = f"Контекст использования, формальный стиль {uuid.uuid4().hex[:4]}"
        data = [{"word": "test", "context_ru": context, "sentence_ru": "Тест"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["context"] == context, "context was not parsed"

    def test_reads_importance_as_string_from_json_entry(self):
        importance = 8
        data = [{"word": "test", "importance": importance, "sentence_ru": "Тест"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["importance"] == str(importance), "importance was not converted to string"

    def _write(self, data):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False)
        return path


class TestVocabularyFiltersInvalidEntries:
    """
    Vocabulary skips entries without required fields
    """

    def test_skips_entry_without_word(self):
        data = [{"sentence_ru": "Предложение без слова"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        with pytest.raises(ValueError):
            vocabulary.entries()

    def test_skips_entry_without_russian_sentence(self):
        data = [{"word": "test"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        with pytest.raises(ValueError):
            vocabulary.entries()

    def test_reads_only_valid_entries_from_mixed_data(self):
        valid = {"word": f"valid_{uuid.uuid4().hex[:4]}", "sentence_ru": "Валидное"}
        invalid = {"word": "invalid"}
        data = [invalid, valid, {"sentence_ru": "Без слова"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert len(entries) == 1, "only one valid entry should be returned"

    def _write(self, data):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False)
        return path


class TestVocabularyCoalescesNullValues:
    """
    Vocabulary coalesces JSON null values to empty strings
    """

    def test_null_pronunciation_becomes_empty_string(self):
        """VocabularyMapping returns empty string when pronunciation is null"""
        data = [{"word": "test", "pronunciation": None, "sentence_ru": "\u041f\u0440\u0435\u0434\u043b\u043e\u0436\u0435\u043d\u0438\u0435"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["pronunciation"] == "", \
            "null pronunciation was not coalesced to empty string"

    def test_null_context_becomes_empty_string(self):
        """VocabularyMapping returns empty string when context_ru is null"""
        data = [{"word": "test", "context_ru": None, "sentence_ru": "\u041f\u0440\u0435\u0434\u043b\u043e\u0436\u0435\u043d\u0438\u0435"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["context"] == "", \
            "null context was not coalesced to empty string"

    def test_null_importance_becomes_empty_string(self):
        """VocabularyMapping returns empty string when importance is null"""
        data = [{"word": "test", "importance": None, "sentence_ru": "\u041f\u0440\u0435\u0434\u043b\u043e\u0436\u0435\u043d\u0438\u0435"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["importance"] == "", \
            "null importance was not coalesced to empty string"

    def _write(self, data):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False)
        return path


class TestVocabularyHandlesSpecialCharacters:
    """
    Vocabulary handles commas, quotes, and special characters that break CSV
    """

    def test_handles_commas_in_sentence(self):
        sentence = "The king said, 'I will punish you,' and left."
        data = [{"word": "king", "sentence_en": sentence, "sentence_ru": "Король"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["example"] == sentence, "commas in sentence broke parsing"

    def test_handles_quotes_in_sentence(self):
        sentence = 'She whispered: "Don\'t go there."'
        data = [{"word": "whisper", "sentence_en": sentence, "sentence_ru": "Шёпот"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["example"] == sentence, "quotes in sentence broke parsing"

    def test_handles_unicode_characters(self):
        sentence = "Он сказал: «Привет» — и ушёл 🎭"
        data = [{"word": "test", "sentence_ru": sentence}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["sentence"] == sentence, "unicode characters broke parsing"

    def _write(self, data):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False)
        return path


class TestVocabularyRejectsNonArrayJson:
    """
    Vocabulary raises ValueError when JSON root is not an array
    """

    def test_rejects_json_object_instead_of_array(self):
        """Vocabulary raises ValueError for a JSON object root"""
        data = {"word": f"слово_{uuid.uuid4().hex[:4]}", "sentence_ru": "Предложение"}
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        with pytest.raises(ValueError):
            vocabulary.entries()

    def test_rejects_json_string_instead_of_array(self):
        """Vocabulary raises ValueError for a JSON string root"""
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(f"строка_{uuid.uuid4().hex[:4]}", f, ensure_ascii=False)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        with pytest.raises(ValueError):
            vocabulary.entries()


class TestVocabularyRejectsEmptyData:
    """
    Vocabulary raises ValueError when no valid entries remain after filtering
    """

    def test_rejects_empty_array(self):
        """Vocabulary raises ValueError for an empty JSON array"""
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump([], f)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        with pytest.raises(ValueError):
            vocabulary.entries()

    def test_rejects_when_all_entries_lack_required_fields(self):
        """Vocabulary raises ValueError when every entry is filtered out"""
        data = [
            {"sentence_ru": f"Без слова {uuid.uuid4().hex[:4]}"},
            {"word": f"без_предложения_{uuid.uuid4().hex[:4]}"},
        ]
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        with pytest.raises(ValueError):
            vocabulary.entries()


class TestGreekMappingReadsExampleFromSentenceEl:
    """
    VocabularyMapping with Greek config reads example from sentence_el field
    """

    def test_reads_greek_example_sentence(self):
        example = f"Η γάτα κάθεται στο τραπέζι {uuid.uuid4().hex[:4]}"
        data = [{"word": "γάτα", "sentence_el": example, "sentence_ru": "Кошка"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_el")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["example"] == example, "Greek example was not read from sentence_el"

    def test_reads_transcription_from_pronunciation_all(self):
        transcription = f"i ɣata kaθete sto trapezi {uuid.uuid4().hex[:4]}"
        data = [{"word": "γάτα", "pronunciation_all": transcription, "sentence_ru": "Кошка"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_el")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["transcription"] == transcription, "transcription was not read from pronunciation_all"

    def test_english_mapping_returns_empty_transcription(self):
        data = [{"word": "test", "sentence_ru": "Тест"}]
        path = self._write(data)
        mapping = VocabularyMapping(("word", "sentence_ru"), "sentence_en")
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["transcription"] == "", "English mapping should return empty transcription"

    def _write(self, data):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False)
        return path
