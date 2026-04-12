#!/usr/bin/env python3
"""Tests for prompt rendering helpers."""

import uuid

from kamishibai.runtime import audio_prompt
from kamishibai.runtime import scene_prompt


class TestAudioPromptUsesSharedTemplate:
    """Audio prompt rendering comes from the shared asset template."""

    def test_renders_language_into_audio_prompt(self):
        language = f"Language_{uuid.uuid4().hex[:4]}"
        result = audio_prompt(language)
        assert result == f"Say in natural {language}: {{text}}", "shared audio prompt was not rendered from the common template"


class TestScenePromptUsesSharedTemplate:
    """Scene prompt rendering comes from the shared asset template."""

    def test_renders_language_into_scene_prompt(self):
        language = f"Language_{uuid.uuid4().hex[:4]}"
        result = scene_prompt(language)
        assert f"educational {language} flashcards" in result, "shared scene prompt did not include the target language"

    def test_preserves_json_schema_braces_for_second_format_pass(self):
        language = f"Language_{uuid.uuid4().hex[:4]}"
        result = scene_prompt(language).format(sentence="demo")
        assert '"x": int' in result, "scene prompt lost JSON schema braces during formatting"
