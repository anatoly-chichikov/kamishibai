"""CLI entry helpers for kamishibai."""

import argparse
import json
import sys
from datetime import datetime
from pathlib import Path

import genanki

from .anki import CardModel
from .anki import StableId
from .anki import VocabularyDeck
from .anki import VocabularyNote
from .config import Fonts
from .config import naming
from .diagnosis import DiagnosisSelector
from .input import Vocabulary
from .input import VocabularyMapping
from .locations import Locations
from .locations import data_home
from .media import Pipeline
from .progress import ProgressSelector
from .report import Report
from .report import Thumbnail
from .report import VocabularyLayout
from .runtime import Media
from .runtime import client


def arguments(argv):
    """Parse CLI arguments for the unified kamishibai application."""
    parser = argparse.ArgumentParser(
        description="Convert vocabulary JSON to an illustrated Anki deck",
        epilog=(
            "Examples:\n"
            "  kamishibai\n"
            "  kamishibai my-words.json\n"
            "  kamishibai --deck \"Greek Basics\" my-words.json\n"
            "  kamishibai --output ./output --cache ~/.cache/kamishibai my-words.json"
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--deck", help="Optional deck name override")
    parser.add_argument("--output", help="Advanced: directory for generated output files")
    parser.add_argument("--cache", help="Advanced: directory for reusable media cache")
    parser.add_argument("path", nargs="?", help="Optional path to vocabulary JSON file")
    return parser.parse_args(argv)


def locations(args):
    """Return the resolved filesystem locations for a CLI invocation."""
    return Locations(args)


def path(args):
    """Resolve the input vocabulary path from CLI arguments or Downloads."""
    return locations(args).input()


def root():
    """Return the default application data directory for compatibility callers."""
    return data_home() / "kamishibai"


def main(argv=None):
    """Run the application logic for the provided CLI arguments."""
    args = arguments(argv)
    resolved = locations(args)
    vocabulary = Vocabulary(resolved.input(), VocabularyMapping())
    document = vocabulary.document()
    item = client()
    decknaming = naming(args)
    media = Media(item, resolved.cache())
    entries = vocabulary.entries(document)
    model = CardModel(StableId(f"{decknaming.name()} Model").value()).model()
    deck = genanki.Deck(StableId(decknaming.name()).value(), decknaming.name())
    container = VocabularyDeck(deck, VocabularyNote(model), [])
    progress = ProgressSelector(sys.stdout.isatty()).selected()
    failed, processed = Pipeline(media, media, container, progress).process(entries)
    output = resolved.output()
    output.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now().strftime("%Y-%m-%d_%H%M%S")
    apkg = output / f"{decknaming.prefix()}_{stamp}.apkg"
    container.save(str(apkg))
    report = Report(VocabularyLayout(), Fonts(), Thumbnail(150))
    for entry, imagepath in processed:
        report.append(entry, imagepath)
    pdf = output / f"{decknaming.prefix()}_{stamp}.pdf"
    report.save(str(pdf))
    progress.result("Anki deck", str(apkg))
    progress.result("Report", str(pdf))
    progress.result("Output", str(output))
    progress.finish(len(entries) - len(failed), len(entries), failed)


def run(argv=None):
    """Execute the CLI and translate failures into process exit codes."""
    try:
        main(sys.argv[1:] if argv is None else argv)
        return 0
    except KeyboardInterrupt:
        return 130
    except (FileNotFoundError, json.JSONDecodeError, ValueError, PermissionError, OSError, EnvironmentError) as error:
        diagnosis = DiagnosisSelector(sys.stderr.isatty()).selected()
        diagnosis.show(str(error), getattr(error, "filename", ""))
        return 1
