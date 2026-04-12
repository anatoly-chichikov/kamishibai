"""Runtime configuration helpers for schema-driven language profiles."""

import re
from typing import final

from .report import FontFamily
from .target import AudioProfile
from .target import DeckNaming
from .target import FontProfile
from .target import ImageProfile
from .target import LanguageProfile
from .target import Profiles
from .target import UiLabels


_DEFAULT_FONT = "DejaVu Sans"
_DEFAULT_LABELS = UiLabels("Translation", "Context", "Hint", "Importance")
_DEFAULT_NAMING = DeckNaming("Kamishibai Deck", "kamishibai-deck", "kamishibai.json")
_PROFILES = Profiles(
    {
        "de": LanguageProfile(
            "de",
            AudioProfile("German", "audio-de"),
            ImageProfile("eng+deu", "manga-de"),
            DeckNaming("German Vocabulary", "de", "kamishibai.json"),
            FontProfile(_DEFAULT_FONT),
            UiLabels("Übersetzung", "Kontext", "Hinweis", "Wichtigkeit"),
        ),
        "el": LanguageProfile(
            "el",
            AudioProfile("Greek", "audio-el"),
            ImageProfile("eng+ell", "manga-el"),
            DeckNaming("Greek Vocabulary", "el", "kamishibai.json"),
            FontProfile(_DEFAULT_FONT),
            UiLabels("Μετάφραση", "Πλαίσιο", "Υπόδειξη", "Σπουδαιότητα"),
        ),
        "en": LanguageProfile(
            "en",
            AudioProfile("English", "audio-en"),
            ImageProfile("eng", "manga-en"),
            DeckNaming("English Vocabulary", "en", "kamishibai.json"),
            FontProfile(_DEFAULT_FONT),
            UiLabels("Translation", "Context", "Hint", "Importance"),
        ),
        "es": LanguageProfile(
            "es",
            AudioProfile("Spanish", "audio-es"),
            ImageProfile("eng+spa", "manga-es"),
            DeckNaming("Spanish Vocabulary", "es", "kamishibai.json"),
            FontProfile(_DEFAULT_FONT),
            UiLabels("Traducción", "Contexto", "Pista", "Importancia"),
        ),
        "ru": LanguageProfile(
            "ru",
            AudioProfile("Russian", "audio-ru"),
            ImageProfile("eng+rus", "manga-ru"),
            DeckNaming("Russian Vocabulary", "ru", "kamishibai.json"),
            FontProfile(_DEFAULT_FONT),
            UiLabels("Перевод", "Контекст", "Подсказка", "Важность"),
        ),
        "zh": LanguageProfile(
            "zh",
            AudioProfile("Mandarin Chinese", "audio-zh"),
            ImageProfile("eng+chi_sim", "manga-zh"),
            DeckNaming("Chinese Vocabulary", "zh", "kamishibai.json"),
            FontProfile("Hiragino Sans GB"),
            UiLabels("翻译", "语境", "提示", "重要性"),
        ),
    },
    "eng",
)


def profiles():
    """Return the configured language registry."""
    return _PROFILES


def profile(code):
    """Return the configured LanguageProfile for a supported code."""
    return profiles().item(code)


def prefix(name):
    """Return a filesystem-friendly prefix derived from the deck name."""
    value = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")
    return value or "deck"


def naming(args, entries):
    """Return the effective deck naming after applying CLI overrides."""
    if args.deck:
        return DeckNaming(args.deck, prefix(args.deck), _DEFAULT_NAMING.default())
    codes = {entry["target_lang"] for entry in entries}
    if len(codes) == 1:
        return profile(next(iter(codes))).naming()
    return _DEFAULT_NAMING


@final
class Fonts:
    """Selects a PDF font family from schema-driven language profiles."""

    def __init__(self, items=None, default=None):
        self._items = items if items is not None else profiles()
        self._default = default if default is not None else _DEFAULT_FONT

    def selected(self, entry):
        """Return the report font family for a single entry."""
        names = self._families(entry)
        for name in names:
            if name != self._default:
                return FontFamily(name)
        if names:
            return FontFamily(names[0])
        return FontFamily(self._default)

    def _families(self, entry):
        """Return configured font family names referenced by the entry."""
        names = []
        for code in (entry.get("source_lang", ""), entry.get("target_lang", "")):
            if not code:
                continue
            try:
                names.append(self._items.item(code).font().report())
            except ValueError:
                continue
        return names


@final
class Labels:
    """Selects user-facing labels from schema-driven source profiles."""

    def __init__(self, items=None, default=None):
        self._items = items if items is not None else profiles()
        self._default = default if default is not None else _DEFAULT_LABELS

    def selected(self, entry):
        """Return UI labels for a single entry."""
        code = entry.get("source_lang", "")
        if not code:
            return self._default
        try:
            return self._items.item(code).labels()
        except ValueError:
            return self._default
