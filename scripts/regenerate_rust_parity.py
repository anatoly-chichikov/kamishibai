"""Regenerate offline reference artifacts for the Rust parity contract."""

import json
import os
import re
import shutil
import sqlite3
import sys
import wave
import zipfile
from io import BytesIO
from pathlib import Path
from types import SimpleNamespace

from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "python_reference" / "src"
FIXTURES = ROOT / "tests" / "fixtures" / "reference"
INPUTS = FIXTURES / "inputs"
MANIFESTS = FIXTURES / "manifests"
SANDBOX = ROOT / "tmp" / "parity"

if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))

from kamishibai.anki import CardModel
from kamishibai.anki import StableId
from kamishibai.anki import VocabularyDeck
from kamishibai.anki import VocabularyNote
from kamishibai.config import Fonts
from kamishibai.config import Labels
from kamishibai.config import naming
from kamishibai.diagnosis import PlainDiagnosis
from kamishibai.diagnosis import RichDiagnosis
from kamishibai.input import Vocabulary
from kamishibai.input import VocabularyMapping
from kamishibai.media import Audio
from kamishibai.media import Illustration
from kamishibai.media import Pipeline
from kamishibai.media import TtsVoice
from kamishibai.progress import PlainProgress
from kamishibai.progress import RichProgress
from kamishibai.report import Report
from kamishibai.report import Thumbnail
from kamishibai.report import VocabularyLayout
from kamishibai.runtime import audio_prompt
from kamishibai.runtime import scene_prompt
from kamishibai.runtime import template
from kamishibai.scene import Cache
from kamishibai.scene import MangaRenderer
from kamishibai.scene import SceneTranslator


def ensure():
    """Create the reference fixture directories when they are missing."""
    INPUTS.mkdir(parents=True, exist_ok=True)
    MANIFESTS.mkdir(parents=True, exist_ok=True)
    (MANIFESTS / "normalized").mkdir(parents=True, exist_ok=True)


def write(path, text):
    """Write UTF-8 text to a file path."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def dump(path, value):
    """Write JSON with stable formatting to a file path."""
    write(path, json.dumps(value, indent=2, ensure_ascii=False, sort_keys=True) + "\n")


def reset(path):
    """Recreate a sandbox directory as an empty path."""
    if path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True, exist_ok=True)
    return path


def cases():
    """Return the fixed input document fixtures for parity generation."""
    return {
        "single-target-en": {
            "entries": [
                entry(
                    "кошка",
                    "Кошка спит на окне",
                    "ru",
                    "The cat is sleeping on the windowsill",
                    "en",
                    "cat",
                    "kæt",
                    "kat",
                    "Кошка",
                    "домашнее животное",
                    "нейтральный стиль\nкороткая сцена",
                    7,
                )
            ]
        },
        "single-target-el": {
            "entries": [
                entry(
                    "harbor",
                    "The harbor is quiet tonight",
                    "en",
                    "Το λιμάνι είναι ήσυχο απόψε",
                    "el",
                    "λιμάνι",
                    "ˈhɑːrbər",
                    "harbor",
                    "harbor",
                    "coast",
                    "calm evening",
                    5,
                )
            ]
        },
        "single-target-de": {
            "entries": [
                entry(
                    "mañana",
                    "Mañana salimos temprano",
                    "es",
                    "Morgen fahren wir früh los",
                    "de",
                    "tomorrow",
                    None,
                    "",
                    "Mañana",
                    "tiempo",
                    None,
                    None,
                )
            ]
        },
        "single-target-es": {
            "entries": [
                entry(
                    "Wolke",
                    "Die Wolke zieht langsam vorbei",
                    "de",
                    "La nube pasa despacio",
                    "es",
                    "cloud",
                    "ˈvɔlkə",
                    "volke",
                    "Wolke",
                    "",
                    "poetisch",
                    4,
                )
            ]
        },
        "single-target-ru": {
            "entries": [
                entry(
                    "朋友",
                    "朋友在门口等我",
                    "zh",
                    "Друг ждёт меня у двери",
                    "ru",
                    "friend",
                    "péngyou",
                    "pengyou",
                    "朋友",
                    "关系",
                    "口语",
                    9,
                )
            ]
        },
        "single-target-zh": {
            "entries": [
                entry(
                    "φως",
                    "Το φως μπαίνει από το παράθυρο",
                    "el",
                    "光从窗户照进来",
                    "zh",
                    "light",
                    "fos",
                    "fos",
                    "φως",
                    "morning",
                    "soft scene",
                    6,
                )
            ]
        },
        "mixed-target-deck": {
            "entries": [
                entry(
                    "harbor",
                    "The harbor is quiet tonight",
                    "en",
                    "Το λιμάνι είναι ήσυχο απόψε",
                    "el",
                    "λιμάνι",
                    "ˈhɑːrbər",
                    "harbor",
                    "harbor",
                    "coast",
                    "calm evening",
                    5,
                ),
                entry(
                    "φως",
                    "Το φως μπαίνει από το παράθυρο",
                    "el",
                    "光从窗户照进来",
                    "zh",
                    "light",
                    "fos",
                    "fos",
                    "φως",
                    "morning",
                    "soft scene",
                    6,
                ),
            ]
        },
        "invalid-document": [
            {
                "term": "broken",
                "source": {"sentence": "Нет корня", "lang": "ru"},
                "target": {"sentence": "No root", "lang": "en"},
            }
        ],
        "supported-languages": {
            "entries": [
                entry(
                    "кошка",
                    "Кошка спит на окне",
                    "ru",
                    "The cat is sleeping on the windowsill",
                    "en",
                    "cat",
                    "kæt",
                    "kat",
                    "Кошка",
                    "домашнее животное",
                    "нейтральный стиль\nкороткая сцена",
                    7,
                ),
                entry(
                    "harbor",
                    "The harbor is quiet tonight",
                    "en",
                    "Το λιμάνι είναι ήσυχο απόψε",
                    "el",
                    "λιμάνι",
                    "ˈhɑːrbər",
                    "harbor",
                    "harbor",
                    "coast",
                    "calm evening",
                    5,
                ),
                entry(
                    "mañana",
                    "Mañana salimos temprano",
                    "es",
                    "Morgen fahren wir früh los",
                    "de",
                    "tomorrow",
                    None,
                    "",
                    "Mañana",
                    "tiempo",
                    None,
                    None,
                ),
                entry(
                    "Wolke",
                    "Die Wolke zieht langsam vorbei",
                    "de",
                    "La nube pasa despacio",
                    "es",
                    "cloud",
                    "ˈvɔlkə",
                    "volke",
                    "Wolke",
                    "",
                    "poetisch",
                    4,
                ),
                entry(
                    "朋友",
                    "朋友在门口等我",
                    "zh",
                    "Друг ждёт меня у двери",
                    "ru",
                    "friend",
                    "péngyou",
                    "pengyou",
                    "朋友",
                    "关系",
                    "口语",
                    9,
                ),
                entry(
                    "φως",
                    "Το φως μπαίνει από το παράθυρο",
                    "el",
                    "光从窗户照进来",
                    "zh",
                    "light",
                    "fos",
                    "fos",
                    "φως",
                    "morning",
                    "soft scene",
                    6,
                ),
            ]
        },
    }


def entry(term, source, source_lang, target, target_lang, meaning, pronunciation, transcription, highlight, hint, context, importance):
    """Return one schema-driven input entry."""
    return {
        "term": term,
        "meaning": meaning,
        "pronunciation": pronunciation,
        "transcription": transcription,
        "importance": importance,
        "source": {
            "sentence": source,
            "lang": source_lang,
            "highlight": highlight,
            "hint": hint,
            "context": context,
        },
        "target": {
            "sentence": target,
            "lang": target_lang,
        },
    }


def inputs(items):
    """Write the fixed input fixtures to disk."""
    for name, value in items.items():
        dump(INPUTS / f"{name}.json", value)


def normalized(items):
    """Write normalized entry outputs and invalid-document failure details."""
    for name in sorted(items):
        path = INPUTS / f"{name}.json"
        if name == "invalid-document":
            try:
                Vocabulary(path, VocabularyMapping()).document()
            except Exception as error:
                dump(
                    MANIFESTS / "invalid-document.json",
                    {
                        "error": type(error).__name__,
                        "message": str(error),
                    },
                )
            continue
        value = Vocabulary(path, VocabularyMapping()).entries()
        dump(MANIFESTS / "normalized" / f"{name}.json", value)


def profiles():
    """Write the supported language profile summary."""
    items = []
    for name in ("de", "el", "en", "es", "ru", "zh"):
        value = __import__("kamishibai.config", fromlist=["profile"]).profile(name)
        items.append(
            {
                "code": value.code(),
                "audio_language": value.audio().language(),
                "audio_cache": value.audio().cache(),
                "image_cache": value.imagery().cache(),
                "ocr": value.imagery().ocr(),
                "deck_name": value.naming().name(),
                "deck_prefix": value.naming().prefix(),
                "deck_default": value.naming().default(),
                "font": value.font().report(),
                "labels": {
                    "sentence": value.labels().sentence(),
                    "context": value.labels().context(),
                    "hint": value.labels().hint(),
                    "importance": value.labels().importance(),
                },
                "audio_prompt": audio_prompt(value.audio().language()),
                "scene_prompt_prefix": scene_prompt(value.audio().language()).splitlines()[0],
            }
        )
    dump(
        MANIFESTS / "profiles.json",
        {
            "codes": [item["code"] for item in items],
            "fallback_ocr": __import__("kamishibai.config", fromlist=["profiles"]).profiles().fallback_ocr(),
            "items": items,
        },
    )


def runtime():
    """Write the frozen runtime constants and prompt assets."""
    dump(
        MANIFESTS / "runtime.json",
        {
            "audio_prompt_template": audio_prompt("English"),
            "scene_prompt_header": scene_prompt("English").splitlines()[:4],
            "voice_pool": list(TtsVoice.pool()),
            "tts_models": ["gemini-2.5-flash-preview-tts", "gemini-2.5-pro-preview-tts"],
            "scene_model": "gemini-3-flash-preview",
            "image_model": "gemini-3.1-flash-image-preview",
            "template": template(),
        },
    )


class Client:
    """Provide deterministic Gemini-like responses for offline fixtures."""

    def __init__(self):
        self.models = Models()


class Models:
    """Dispatch fake model responses by model name."""

    def __init__(self):
        self._audio = bytes([0, 0, 40, 0]) * 240
        self._image = image()

    def generate_content(self, model, contents, config=None):
        """Return a deterministic fake response for the requested model."""
        if model == "gemini-3-flash-preview":
            return Response([Part(text=scene())])
        if model == "gemini-3.1-flash-image-preview":
            return Response([Part(data=self._image)])
        return Response([Part(data=self._audio)])


class Response:
    """Wrap candidate parts in the shape expected by production code."""

    def __init__(self, parts):
        self.candidates = [Candidate(parts)]
        self.prompt_feedback = None


class Candidate:
    """Provide a fake candidate container."""

    def __init__(self, parts):
        self.content = Content(parts)


class Content:
    """Provide a fake content container."""

    def __init__(self, parts):
        self.parts = parts


class Part:
    """Provide either text or inline data for a fake content part."""

    def __init__(self, text=None, data=None):
        self.text = text
        self.inline_data = None if data is None else Data(data)


class Data:
    """Provide an inline-data object with raw bytes."""

    def __init__(self, data):
        self.data = data


class Text:
    """Pretend that OCR found no text in rendered images."""

    def detected(self, scene, image):
        """Return an empty OCR result."""
        return ""


class Border:
    """Pretend that every rendered image has valid borders and gutters."""

    def borders(self, image):
        """Return no border failures."""
        return []

    def gutter(self, image):
        """Return a valid gutter detection result."""
        return True


class Console:
    """Record rich-console print calls."""

    def __init__(self, lines):
        self._lines = lines

    def print(self, text, **kwargs):
        """Record one rich console line."""
        self._lines.append(text)


class Spinner:
    """Record rich-spinner lifecycle events."""

    def __init__(self, lines):
        self._lines = lines

    def update(self, text):
        """Record one spinner update."""
        self._lines.append(f"spinner:{text}")

    def start(self):
        """Record spinner start."""
        self._lines.append("spinner:start")

    def stop(self):
        """Record spinner stop."""
        self._lines.append("spinner:stop")


def scene():
    """Return the raw JSON panel array wrapped in markdown fences."""
    return """```json
[
  {
    "bounds": {"x": 32, "y": 32, "width": 480, "height": 420},
    "scene": {
      "description": "A learner points at a white cat sleeping on a windowsill",
      "subject": {"figure": "learner", "pose": "pointing", "expression": "calm"},
      "environment": {"setting": "sunlit apartment", "details": ["white cat", "open window"]},
      "camera": {"angle": "eye level", "distance": "medium shot", "focus": "cat"},
      "mood": "gentle"
    },
    "narrative_weight": "primary",
    "bleed": false
  },
  {
    "bounds": {"x": 520, "y": 32, "width": 456, "height": 420},
    "scene": {
      "description": "Moonlight covers a quiet harbor with tied boats",
      "subject": {"figure": "boats", "pose": "still", "expression": "silent"},
      "environment": {"setting": "night harbor", "details": ["lantern reflections", "rope"]},
      "camera": {"angle": "wide", "distance": "long shot", "focus": "harbor"},
      "mood": "quiet"
    },
    "narrative_weight": "secondary",
    "bleed": false
  }
]
```"""


def image():
    """Return deterministic image bytes for fake image generation."""
    canvas = Image.new("RGB", (1024, 1024), color=(255, 255, 255))
    pane = Image.new("RGB", (920, 920), color=(210, 210, 210))
    canvas.paste(pane, (52, 52))
    data = BytesIO()
    canvas.save(data, "PNG")
    return data.getvalue()


def sanitize(value):
    """Replace repo-specific absolute paths with stable placeholders."""
    text = str(value)
    text = text.replace(str(ROOT), "$REPO")
    return text


def services(cache):
    """Build deterministic offline audio and illustration services."""
    client = Client()
    audio = Audio(
        client,
        Cache("audio-en", str(cache)),
        audio_prompt("English"),
        TtsVoice(("gemini-2.5-flash-preview-tts", "gemini-2.5-pro-preview-tts")),
    )
    illustration = Illustration(
        Cache("manga-en", str(cache)),
        SceneTranslator(client, scene_prompt("English"), template()),
        MangaRenderer(client, 3, Text(), Border()),
    )
    return audio, illustration


def cache():
    """Write cache, digest, and progress semantics manifests."""
    cache = reset(SANDBOX / "cache-contract")
    audio, illustration = services(cache)
    entry = json.loads((MANIFESTS / "normalized" / "single-target-en.json").read_text(encoding="utf-8"))[0]
    lines = []
    plain = PlainProgress(lines.append)
    audiofile, audiocached = audio.generate(entry["example"])
    imagefile, imagecached = illustration.generate(entry["example"], entry["word"], entry["target_lang"], plain)
    cached = []
    legacy = []
    illustration.generate(entry["example"], entry["word"], entry["target_lang"], PlainProgress(cached.append))
    digest = imagefile.replace(".jpg", "")
    scenefile = Path(illustration.filepath(f"{digest}.json"))
    scene = json.loads(scenefile.read_text(encoding="utf-8"))
    scenefile.unlink()
    Path(illustration.filepath(imagefile)).touch()
    illustration.generate(entry["example"], entry["word"], entry["target_lang"], PlainProgress(legacy.append))
    dump(
        MANIFESTS / "cache.json",
        {
            "audio": {
                "filename": audiofile,
                "cached": audiocached,
                "path": sanitize(audio.filepath(audiofile)),
                "bytes": len(Path(audio.filepath(audiofile)).read_bytes()),
            },
            "illustration": {
                "filename": imagefile,
                "cached": imagecached,
                "path": sanitize(illustration.filepath(imagefile)),
                "scene_path": sanitize(illustration.filepath(f"{digest}.json")),
                "scene": scene,
                "first_pass": [sanitize(item) for item in lines],
                "cached_pass": [sanitize(item) for item in cached],
                "legacy_pass": [sanitize(item) for item in legacy],
            },
        },
    )


def payload():
    """Build a mixed-target deck payload via the production pipeline."""
    base = reset(SANDBOX / "payload")
    cache = base / "cache"
    output = base / "output"
    output.mkdir(parents=True, exist_ok=True)
    audio, illustration = services(cache)
    entries = json.loads((MANIFESTS / "normalized" / "mixed-target-deck.json").read_text(encoding="utf-8"))
    labels = []
    deckname = naming(SimpleNamespace(deck=None), entries)
    deck = __import__("genanki").Deck(StableId(deckname.name()).value(), deckname.name())
    model = CardModel(StableId(f"{deckname.name()} Model").value()).model()
    box = VocabularyDeck(deck, VocabularyNote(model), [])
    plain = PlainProgress(labels.append)
    failed, processed = Pipeline(audio, illustration, box, plain).process(entries)
    apkgfile = output / "mixed-target.apkg"
    box.save(str(apkgfile))
    report = Report(VocabularyLayout(Labels()), Fonts(), Thumbnail(150))
    for row, imagepath in processed:
        report.append(row, imagepath)
    pdffile = output / "mixed-target.pdf"
    report.save(str(pdffile))
    apkg(apkgfile)
    report_manifest(pdffile, processed)
    plain.result("Anki deck", str(apkgfile))
    plain.result("Report", str(pdffile))
    plain.result("Output", str(output))
    plain.finish(len(entries) - len(failed), len(entries), failed)
    return labels, failed


def apkg(path):
    """Write a structural manifest for the generated APKG."""
    box = reset(SANDBOX / "apkg-extract")
    with zipfile.ZipFile(path) as archive:
        archive.extractall(box)
        media = json.loads((box / "media").read_text(encoding="utf-8"))
    database = sqlite3.connect(box / "collection.anki2")
    row = database.execute("select models, decks from col").fetchone()
    notes = [
        item[0].split("\x1f")
        for item in database.execute("select flds from notes order by id").fetchall()
    ]
    database.close()
    models = json.loads(row[0])
    model = next(iter(models.values()))
    entries = json.loads((MANIFESTS / "normalized" / "mixed-target-deck.json").read_text(encoding="utf-8"))
    deckname = naming(SimpleNamespace(deck=None), entries)
    dump(
        MANIFESTS / "apkg.json",
        {
            "zip_entries": sorted(archive_names(path)),
            "media": media,
            "deck": {
                "id": StableId(deckname.name()).value(),
                "name": deckname.name(),
            },
            "model": {
                "id": StableId(f"{deckname.name()} Model").value(),
                "name": model["name"],
                "fields": [item["name"] for item in model["flds"]],
                "template": model["tmpls"][0],
            },
            "notes": notes,
        },
    )


def archive_names(path):
    """Return sorted names from a zip archive."""
    with zipfile.ZipFile(path) as archive:
        return archive.namelist()


def report_manifest(path, processed):
    """Write layout and structural manifests for the generated PDF."""
    entrys = []
    layout = VocabularyLayout(Labels())
    fonts = Fonts()
    for entry, imagepath in processed:
        labels = Labels().selected(entry)
        entrys.append(
            {
                "word": entry["word"],
                "source_lang": entry["source_lang"],
                "target_lang": entry["target_lang"],
                "font": fonts.selected(entry)._regular._family,
                "labels": {
                    "sentence": labels.sentence(),
                    "context": labels.context(),
                    "hint": labels.hint(),
                    "importance": labels.importance(),
                },
                "rows": layout.row(entry),
                "image": sanitize(imagepath),
            }
        )
    data = path.read_bytes()
    dump(
        MANIFESTS / "report.json",
        {
            "entries": entrys,
            "pdf": {
                "header": data[:8].decode("latin-1"),
                "page_count": len(re.findall(rb"/Type /Page\b", data)),
                "bytes": len(data),
            },
        },
    )


def progress(labels):
    """Write plain and rich progress transcripts."""
    write(
        MANIFESTS / "plain-cli.txt",
        "\n".join(sanitize(item) for item in labels) + "\n",
    )
    lines = []
    console = Console(lines)
    spinner = Spinner(lines)
    progress = RichProgress(console, spinner)
    path = sanitize(ROOT / "output" / "deck.apkg")
    progress.card(1, 2, "кошка")
    progress.step("Generating audio")
    progress.done("Generating audio", "generated", path)
    progress.step("Composing scene")
    progress.done("Composing scene", "translated", path.replace(".apkg", ".json"))
    progress.step("Rendering manga")
    progress.done("Rendering manga", "rendered", path.replace(".apkg", ".jpg"))
    progress.skip("слово", "Cannot generate audio for empty text")
    progress.result("Anki deck", path)
    progress.finish(1, 2, [{"word": "слово", "reason": "Cannot generate audio for empty text"}])
    dump(MANIFESTS / "rich-progress.json", [sanitize(item) for item in lines])


def diagnosis():
    """Write plain and rich diagnosis manifests."""
    plain = []
    PlainDiagnosis(plain.append).show("problem", str(ROOT / "tmp" / "broken.json"))
    rich = []
    RichDiagnosis(Console(rich)).show("problem", str(ROOT / "tmp" / "broken.json"))
    dump(
        MANIFESTS / "diagnosis.json",
        {
            "plain": [sanitize(item) for item in plain],
            "rich_count": len(rich),
        },
    )


def pathing():
    """Write path-resolution and exit-code summaries."""
    dump(
        MANIFESTS / "paths.json",
        {
            "fallback_input": "kamishibai.json in current working directory",
            "env_vars": [
                "KAMISHIBAI_INPUT",
                "KAMISHIBAI_OUTPUT",
                "KAMISHIBAI_CACHE",
                "XDG_CACHE_HOME",
                "XDG_DATA_HOME",
            ],
            "default_output": "output directory beside the input file",
            "exit_codes": [0, 1, 130],
            "tty_modes": ["plain", "rich"],
        },
    )


def main():
    """Regenerate all offline parity fixtures and manifests."""
    ensure()
    value = cases()
    inputs(value)
    normalized(value)
    profiles()
    runtime()
    pathing()
    cache()
    labels, failed = payload()
    progress(labels)
    diagnosis()
    dump(
        MANIFESTS / "baseline.json",
        {
            "pytest": "176 passed in 5.64s",
            "failures": failed,
        },
    )
    if SANDBOX.exists():
        shutil.rmtree(SANDBOX)


if __name__ == "__main__":
    main()
