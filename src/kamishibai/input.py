"""Input document loading for kamishibai."""

import json
from typing import Protocol, final


class FieldMapping(Protocol):
    """Protocol for mapping JSON row to normalized entry dict."""

    def mapped(self, row):
        """Return normalized entry dict or None if row is invalid."""
        ...


@final
class VocabularyMapping:
    """Maps vocabulary JSON rows to normalized entry dicts."""

    def mapped(self, row):
        """Return normalized entry dict or None if row is invalid."""
        if not isinstance(row, dict):
            return None
        source = row.get("source")
        target = row.get("target")
        if not isinstance(source, dict) or not isinstance(target, dict):
            return None
        if not row.get("term") or not source.get("sentence") or not source.get("lang") or not target.get("sentence") or not target.get("lang"):
            return None
        return {
            "word": row["term"],
            "pronunciation": row.get("pronunciation") or "",
            "translation": row.get("meaning") or "",
            "example": target["sentence"],
            "source_lang": source["lang"],
            "target_lang": target["lang"],
            "sentence": source["sentence"],
            "highlight": source.get("highlight") or "",
            "hint": source.get("hint") or "",
            "context": source.get("context") or "",
            "importance": str(row.get("importance") or ""),
            "transcription": row.get("transcription") or "",
        }


@final
class Vocabulary:
    """Reads vocabulary entries from a JSON file."""

    def __init__(self, path, mapping):
        self._path = path
        self._mapping = mapping

    def document(self):
        """Load and validate the root JSON document."""
        with open(self._path, "r", encoding="utf-8") as file:
            data = json.load(file)
        if not isinstance(data, dict):
            raise ValueError(
                f"Expected a JSON object in '{self._path}' but found {type(data).__name__}"
            )
        if not isinstance(data.get("entries"), list):
            raise ValueError(
                f"Expected an 'entries' array in '{self._path}'"
            )
        return data

    def entries(self, document=None):
        """Load, filter, and return vocabulary entries."""
        data = self.document() if document is None else document
        result = []
        for row in data["entries"]:
            entry = self._mapping.mapped(row)
            if entry is not None:
                result.append(entry)
        if not result:
            raise ValueError(
                f"No valid entries found in '{self._path}'; each entry requires 'term', 'source.sentence', 'source.lang', 'target.sentence', and 'target.lang'"
            )
        return result
