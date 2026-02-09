#!/usr/bin/env python3
"""
Unit tests for Vocabulary class
"""

import json
import os
import tempfile
import uuid

import pytest

from create_anki_deck import EnglishMapping
from deck import Vocabulary


class TestVocabularyReadsValidEntry:
    """
    Vocabulary correctly parses valid vocabulary entries
    """

    def test_reads_word_from_json_entry(self):
        word = f"testword_{uuid.uuid4().hex[:8]}"
        data = [{"word": word, "sentence_ru": "Тестовое предложение"}]
        path = self._write(data)
        mapping = EnglishMapping(("word", "sentence_ru"))
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["word"] == word, "word field was not parsed correctly"

    def test_reads_pronunciation_from_json_entry(self):
        pronunciation = f"/ˈtɛst_{uuid.uuid4().hex[:4]}/"
        data = [{"word": "test", "pronunciation": pronunciation, "sentence_ru": "Тест"}]
        path = self._write(data)
        mapping = EnglishMapping(("word", "sentence_ru"))
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["pronunciation"] == pronunciation, "pronunciation was not parsed"

    def test_reads_translation_from_json_entry(self):
        translation = f"перевод_{uuid.uuid4().hex[:6]}"
        data = [{"word": "test", "translation_ru": translation, "sentence_ru": "Тест"}]
        path = self._write(data)
        mapping = EnglishMapping(("word", "sentence_ru"))
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["translation"] == translation, "translation was not parsed"

    def test_reads_example_sentence_from_json_entry(self):
        example = f"Example with comma, and quotes '{uuid.uuid4().hex[:4]}'"
        data = [{"word": "test", "sentence_en": example, "sentence_ru": "Тест"}]
        path = self._write(data)
        mapping = EnglishMapping(("word", "sentence_ru"))
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["example"] == example, "example sentence was not parsed"

    def test_reads_russian_sentence_from_json_entry(self):
        sentence = f"Русское предложение с запятой, и «кавычками» {uuid.uuid4().hex[:4]}"
        data = [{"word": "test", "sentence_ru": sentence}]
        path = self._write(data)
        mapping = EnglishMapping(("word", "sentence_ru"))
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["sentence"] == sentence, "Russian sentence was not parsed"

    def test_reads_context_from_json_entry(self):
        context = f"Контекст использования, формальный стиль {uuid.uuid4().hex[:4]}"
        data = [{"word": "test", "context_ru": context, "sentence_ru": "Тест"}]
        path = self._write(data)
        mapping = EnglishMapping(("word", "sentence_ru"))
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["context"] == context, "context was not parsed"

    def test_reads_importance_as_string_from_json_entry(self):
        importance = 8
        data = [{"word": "test", "importance": importance, "sentence_ru": "Тест"}]
        path = self._write(data)
        mapping = EnglishMapping(("word", "sentence_ru"))
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
        mapping = EnglishMapping(("word", "sentence_ru"))
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert len(entries) == 0, "entry without word should be skipped"

    def test_skips_entry_without_russian_sentence(self):
        data = [{"word": "test"}]
        path = self._write(data)
        mapping = EnglishMapping(("word", "sentence_ru"))
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert len(entries) == 0, "entry without sentence_ru should be skipped"

    def test_reads_only_valid_entries_from_mixed_data(self):
        valid = {"word": f"valid_{uuid.uuid4().hex[:4]}", "sentence_ru": "Валидное"}
        invalid = {"word": "invalid"}
        data = [invalid, valid, {"sentence_ru": "Без слова"}]
        path = self._write(data)
        mapping = EnglishMapping(("word", "sentence_ru"))
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert len(entries) == 1, "only one valid entry should be returned"

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
        mapping = EnglishMapping(("word", "sentence_ru"))
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["example"] == sentence, "commas in sentence broke parsing"

    def test_handles_quotes_in_sentence(self):
        sentence = 'She whispered: "Don\'t go there."'
        data = [{"word": "whisper", "sentence_en": sentence, "sentence_ru": "Шёпот"}]
        path = self._write(data)
        mapping = EnglishMapping(("word", "sentence_ru"))
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["example"] == sentence, "quotes in sentence broke parsing"

    def test_handles_unicode_characters(self):
        sentence = "Он сказал: «Привет» — и ушёл 🎭"
        data = [{"word": "test", "sentence_ru": sentence}]
        path = self._write(data)
        mapping = EnglishMapping(("word", "sentence_ru"))
        vocabulary = Vocabulary(path, mapping)
        entries = vocabulary.entries()
        assert entries[0]["sentence"] == sentence, "unicode characters broke parsing"

    def _write(self, data):
        fd, path = tempfile.mkstemp(suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False)
        return path
