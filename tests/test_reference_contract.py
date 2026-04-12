#!/usr/bin/env python3
"""Tests for frozen Rust parity reference artifacts."""

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests" / "fixtures" / "reference" / "manifests"


def _json(name):
    """Return parsed JSON from a reference manifest."""
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


class TestReferenceBaseline:
    """Reference manifests keep the Python test baseline frozen."""

    def test_keeps_the_documented_pytest_result(self):
        """Reference baseline keeps the recorded pytest result string."""
        assert _json("baseline.json")["pytest"] == "176 passed in 5.64s", "reference baseline drifted away from the recorded pytest result"


class TestReferenceNormalizedShape:
    """Reference normalized fixtures keep the full entry contract."""

    def test_keeps_all_twelve_normalized_fields(self):
        """Reference normalized entry keeps the full key set."""
        assert sorted(_json("normalized/single-target-en.json")[0].keys()) == ["context", "example", "highlight", "hint", "importance", "pronunciation", "sentence", "source_lang", "target_lang", "transcription", "translation", "word"], "reference normalized shape lost a required field"


class TestReferenceApkgContract:
    """Reference APKG manifest keeps the note model and deck naming contract."""

    def test_keeps_the_mixed_target_default_deck_name(self):
        """Reference APKG manifest keeps the mixed-target deck fallback name."""
        assert _json("apkg.json")["deck"]["name"] == "Kamishibai Deck", "reference APKG deck name no longer matches the mixed-target fallback"

    def test_keeps_the_eleven_note_fields_in_order(self):
        """Reference APKG manifest keeps the exact field order."""
        assert _json("apkg.json")["model"]["fields"] == ["SourceSentence", "Term", "Pronunciation", "Meaning", "TargetSentence", "Importance", "Audio", "Illustration", "Hint", "Context", "PronunciationAll"], "reference APKG fields no longer match the frozen field order"


class TestReferenceReportContract:
    """Reference report manifest keeps the font and label asymmetry visible."""

    def test_keeps_the_chinese_font_selection_case(self):
        """Reference report manifest keeps Hiragino for zh-target entries."""
        assert [item["font"] for item in _json("report.json")["entries"] if item["target_lang"] == "zh"] == ["Hiragino Sans GB"], "reference report lost the zh font-selection case"
