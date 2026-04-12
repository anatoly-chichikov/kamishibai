"""Runtime configuration helpers for kamishibai."""

import re
from typing import final

from .report import FontFamily
from .target import AudioProfile
from .target import DeckNaming
from .target import ImageProfile
from .target import TargetProfile


def profile(code):
    """Build the configured TargetProfile for a supported code."""
    profiles = {
        "de": TargetProfile(
            "de",
            AudioProfile("German", "audio-de"),
            ImageProfile("eng+deu", "manga-de"),
            DeckNaming("German Vocabulary", "de", "kamishibai.json"),
        ),
        "el": TargetProfile(
            "el",
            AudioProfile("Greek", "audio-el"),
            ImageProfile("eng+ell", "manga-el"),
            DeckNaming("Greek Vocabulary", "el", "kamishibai.json"),
        ),
        "en": TargetProfile(
            "en",
            AudioProfile("English", "audio-en"),
            ImageProfile("eng", "manga-en"),
            DeckNaming("English Vocabulary", "en", "kamishibai.json"),
        ),
        "es": TargetProfile(
            "es",
            AudioProfile("Spanish", "audio-es"),
            ImageProfile("eng+spa", "manga-es"),
            DeckNaming("Spanish Vocabulary", "es", "kamishibai.json"),
        ),
        "zh": TargetProfile(
            "zh",
            AudioProfile("Mandarin Chinese", "audio-zh"),
            ImageProfile("eng+chi_sim", "manga-zh"),
            DeckNaming("Chinese Vocabulary", "zh", "kamishibai.json"),
        ),
    }
    if code in profiles:
        return profiles[code]
    raise ValueError(f"Unsupported target language '{code}'")


def prefix(name):
    """Return a filesystem-friendly prefix derived from the deck name."""
    value = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")
    return value or "deck"


def naming(args):
    """Return the effective deck naming after applying CLI overrides."""
    name = args.deck if args.deck else "Kamishibai Deck"
    return DeckNaming(name, prefix(name), "kamishibai.json")


@final
class Fonts:
    """Selects a PDF font family for each processed entry."""

    def __init__(self):
        self._default = FontFamily("DejaVu Sans")
        self._cjk = FontFamily("Hiragino Sans GB")

    def selected(self, entry):
        """Return the report font family for a single entry."""
        if entry.get("target_lang") == "zh":
            return self._cjk
        return self._default
