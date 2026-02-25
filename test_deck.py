#!/usr/bin/env python3
"""
Tests for deck module
"""

import hashlib
import json
import logging
import os
import tempfile
import uuid

import pytest
from PIL import Image

from deck import CardModel
from deck import Illustration
from deck import StableId
from deck import TtsVoice
from deck import VocabularyNote

logging.disable(logging.CRITICAL)


class FakeCache:
    """In-memory cache backed by a temporary directory"""

    def __init__(self, directory):
        self._path = directory

    def exists(self, filename):
        """Check if file exists in temp directory"""
        return os.path.exists(os.path.join(self._path, filename))

    def filepath(self, filename):
        """Return full path in temp directory"""
        return os.path.join(self._path, filename)

    def stage(self, suffix):
        """Return a temporary file path for atomic writes"""
        import tempfile as tf
        fd, path = tf.mkstemp(suffix=suffix, dir=self._path)
        os.close(fd)
        return path

    def commit(self, staged, filename):
        """Atomically move staged file to final cache path"""
        os.replace(staged, os.path.join(self._path, filename))


class FakeTranslator:
    """Records translation calls and returns a fixed scene"""

    def __init__(self, scene):
        self._scene = scene
        self._calls = []

    def translate(self, sentence):
        """Record call and return fixed scene"""
        self._calls.append(sentence)
        return self._scene


class FakeRenderer:
    """Returns a gray image without calling any API"""

    def __init__(self, pixels):
        self._pixels = pixels

    def render(self, scene, word, progress):
        """Return a synthetic grayscale image"""
        return Image.new("L", (self._pixels, self._pixels), 128)


class FakeProgress:
    """Collects progress events into a list"""

    def __init__(self, lines):
        self._lines = lines

    def card(self, index, total, word):
        """Record card event"""
        self._lines.append(("card", index, total, word))

    def step(self, name):
        """Record step event"""
        self._lines.append(("step", name))

    def done(self, name, label, path=""):
        """Record done event"""
        self._lines.append(("done", name, label, path))

    def retry(self, name, attempt, reason):
        """Record retry event"""
        self._lines.append(("retry", name, attempt, reason))

    def skip(self, word, reason):
        """Record skip event"""
        self._lines.append(("skip", word, reason))


class TestStableIdProducesDeterministicValue:
    """StableId produces the same integer for the same name across invocations"""

    def test_same_name_returns_same_value(self):
        """StableId returns identical values for the same name string"""
        name = f"Deck-{uuid.uuid4().hex[:8]}"
        assert StableId(name).value() == StableId(name).value(), \
            "same name produced different IDs"

    def test_different_names_return_different_values(self):
        """StableId returns different values for distinct name strings"""
        first = StableId(f"English-{uuid.uuid4().hex[:8]}").value()
        second = StableId(f"Greek-{uuid.uuid4().hex[:8]}").value()
        assert first != second, \
            "different names produced the same ID"

    def test_value_fits_in_31_bits(self):
        """StableId value fits within genanki 31-bit range"""
        name = f"\u041a\u0438\u0440\u0438\u043b\u043b\u0438\u0446\u0430-{uuid.uuid4().hex[:8]}"
        result = StableId(name).value()
        assert 0 <= result < (1 << 31), \
            "value exceeds 31-bit range required by genanki"


class _FailingRenderer:
    """Renderer that always raises after being called"""

    def render(self, scene, word, progress):
        """Raise to simulate a rendering failure"""
        raise RuntimeError(f"simulated render failure for '{word}'")


class TestIllustrationDoesntCacheOnFailure:
    """Illustration does not leave partial files in cache when rendering fails"""

    def test_no_image_cached_after_render_failure(self):
        """Illustration leaves no image file when renderer raises"""
        directory = tempfile.mkdtemp()
        panel = {"id": uuid.uuid4().hex}
        scene = {"manga_panel": {"panels": [panel], "meta": {"title": "t", "description": "d"}}}
        sentence = f"\u00e9chec-{uuid.uuid4().hex[:6]}"
        digest = hashlib.md5(sentence.encode()).hexdigest()[:12]
        imagepath = os.path.join(directory, f"{digest}.jpg")
        cache = FakeCache(directory)
        translator = FakeTranslator(scene)
        renderer = _FailingRenderer()
        progress = FakeProgress([])
        illustration = Illustration(cache, translator, renderer)
        try:
            illustration.generate(sentence, f"w-{uuid.uuid4().hex[:4]}", progress)
        except RuntimeError:
            pass
        assert not os.path.exists(imagepath), \
            "partial image file was left in cache after render failure"


class TestIllustrationSceneCache:
    """Tests for scene JSON caching in Illustration"""

    def test_saves_scene_json_when_generating(self):
        """Illustration saves scene JSON file alongside the manga image"""
        directory = tempfile.mkdtemp()
        panel = {"id": uuid.uuid4().hex}
        scene = {"manga_panel": {"panels": [panel], "meta": {"title": "t", "description": "d"}}}
        sentence = f"\u00e9preuve-{uuid.uuid4().hex[:6]}"
        digest = hashlib.md5(sentence.encode()).hexdigest()[:12]
        scenepath = os.path.join(directory, f"{digest}.json")
        cache = FakeCache(directory)
        translator = FakeTranslator(scene)
        renderer = FakeRenderer(64)
        progress = FakeProgress([])
        illustration = Illustration(cache, translator, renderer)
        illustration.generate(sentence, f"w-{uuid.uuid4().hex[:4]}", progress)
        assert os.path.isfile(scenepath), "scene JSON was not written to cache"

    def test_loads_cached_scene_without_translating(self):
        """Illustration skips translator when scene JSON already exists in cache"""
        directory = tempfile.mkdtemp()
        panel = {"id": uuid.uuid4().hex}
        scene = {"manga_panel": {"panels": [panel], "meta": {"title": "t", "description": "d"}}}
        sentence = f"\u03b1\u03c1\u03c7\u03ae-{uuid.uuid4().hex[:6]}"
        digest = hashlib.md5(sentence.encode()).hexdigest()[:12]
        scenepath = os.path.join(directory, f"{digest}.json")
        with open(scenepath, "w", encoding="utf-8") as handle:
            json.dump(scene, handle)
        translator = FakeTranslator(scene)
        cache = FakeCache(directory)
        renderer = FakeRenderer(64)
        progress = FakeProgress([])
        illustration = Illustration(cache, translator, renderer)
        illustration.generate(sentence, f"w-{uuid.uuid4().hex[:4]}", progress)
        assert len(translator._calls) == 0, "translator was called despite cached scene"

    def test_reports_scene_path_in_progress(self):
        """Illustration includes scene file path in progress done label"""
        directory = tempfile.mkdtemp()
        panel = {"id": uuid.uuid4().hex}
        scene = {"manga_panel": {"panels": [panel], "meta": {"title": "t", "description": "d"}}}
        sentence = f"S\u00e4tz-{uuid.uuid4().hex[:6]}"
        digest = hashlib.md5(sentence.encode()).hexdigest()[:12]
        scenepath = os.path.join(directory, f"{digest}.json")
        events = []
        cache = FakeCache(directory)
        translator = FakeTranslator(scene)
        renderer = FakeRenderer(64)
        progress = FakeProgress(events)
        illustration = Illustration(cache, translator, renderer)
        illustration.generate(sentence, f"w-{uuid.uuid4().hex[:4]}", progress)
        paths = [e[3] for e in events if e[0] == "done" and e[1] == "Composing scene"]
        assert scenepath == paths[0], "scene path was not reported in progress"

    def test_cached_scene_reports_cached_label(self):
        """Illustration reports cached label when loading scene from file"""
        directory = tempfile.mkdtemp()
        panel = {"id": uuid.uuid4().hex}
        scene = {"manga_panel": {"panels": [panel], "meta": {"title": "t", "description": "d"}}}
        sentence = f"\u0442\u0435\u0441\u0442-{uuid.uuid4().hex[:6]}"
        digest = hashlib.md5(sentence.encode()).hexdigest()[:12]
        scenepath = os.path.join(directory, f"{digest}.json")
        with open(scenepath, "w", encoding="utf-8") as handle:
            json.dump(scene, handle)
        events = []
        cache = FakeCache(directory)
        translator = FakeTranslator(scene)
        renderer = FakeRenderer(64)
        progress = FakeProgress(events)
        illustration = Illustration(cache, translator, renderer)
        illustration.generate(sentence, f"w-{uuid.uuid4().hex[:4]}", progress)
        labels = [e[2] for e in events if e[0] == "done" and e[1] == "Composing scene"]
        assert labels[0] == "cached", "scene label did not indicate cache hit"

    def test_omits_scene_path_when_legacy_cache_lacks_json(self):
        """Illustration omits scene path when image cached but scene JSON missing"""
        directory = tempfile.mkdtemp()
        sentence = f"legacy-{uuid.uuid4().hex[:6]}"
        digest = hashlib.md5(sentence.encode()).hexdigest()[:12]
        imagepath = os.path.join(directory, f"{digest}.jpg")
        Image.new("L", (64, 64), 128).save(imagepath, "JPEG")
        events = []
        cache = FakeCache(directory)
        scene = {"manga_panel": {"panels": [{"id": "x"}], "meta": {"title": "t", "description": "d"}}}
        translator = FakeTranslator(scene)
        renderer = FakeRenderer(64)
        progress = FakeProgress(events)
        illustration = Illustration(cache, translator, renderer)
        illustration.generate(sentence, f"w-{uuid.uuid4().hex[:4]}", progress)
        paths = [e[3] for e in events if e[0] == "done" and e[1] == "Composing scene"]
        assert paths[0] == "", "scene path was shown for legacy cache without JSON"


class TestVocabularyNoteProducesElevenFields:
    """VocabularyNote produces a note with 11 fields"""

    def test_note_has_eleven_fields(self):
        """VocabularyNote creates a genanki Note with exactly 11 fields"""
        model = CardModel(StableId(f"Model-{uuid.uuid4().hex[:8]}").value()).model()
        note = VocabularyNote(model)
        entry = {
            "word": f"wörd_{uuid.uuid4().hex[:4]}",
            "pronunciation": "ˈtɛst",
            "translation": "тéст",
            "example": "A séntence",
            "sentence": "Предложéние",
            "highlight": "",
            "hint": "подскáзка",
            "context": "контéкст",
            "importance": "7",
        }
        result = note.note(entry, "[sound:x.wav]", "<img src='x.jpg'>")
        assert len(result.fields) == 11, "note did not produce exactly 11 fields"

    def test_last_field_empty_without_transcription(self):
        """VocabularyNote leaves PronunciationAll empty when transcription absent"""
        model = CardModel(StableId(f"Model-{uuid.uuid4().hex[:8]}").value()).model()
        note = VocabularyNote(model)
        entry = {
            "word": f"wörd_{uuid.uuid4().hex[:4]}",
            "pronunciation": "ˈtɛst",
            "translation": "тéст",
            "example": "A séntence",
            "sentence": "Предложéние",
            "highlight": "",
            "hint": "",
            "context": "",
            "importance": "5",
        }
        result = note.note(entry, "[sound:x.wav]", "<img src='x.jpg'>")
        assert result.fields[10] == "", "PronunciationAll should be empty without transcription"

    def test_last_field_formatted_with_transcription(self):
        """VocabularyNote formats PronunciationAll when transcription is present"""
        model = CardModel(StableId(f"Model-{uuid.uuid4().hex[:8]}").value()).model()
        note = VocabularyNote(model)
        transcription = f"i ɣata {uuid.uuid4().hex[:4]}"
        entry = {
            "word": f"γάτα_{uuid.uuid4().hex[:4]}",
            "pronunciation": "ˈɣata",
            "translation": "кошка",
            "example": "Η γάτα κάθεται",
            "sentence": "Кошка сидит",
            "highlight": "",
            "hint": "",
            "context": "",
            "importance": "6",
            "transcription": transcription,
        }
        result = note.note(entry, "[sound:x.wav]", "<img src='x.jpg'>")
        assert result.fields[10] == f"/{transcription}/", "PronunciationAll was not formatted with slashes"


class TestTtsVoiceSelectsRandomly:
    """TtsVoice selects a random voice from the full Gemini pool on each call"""

    def test_returns_valid_speech_config(self):
        """TtsVoice produces a SpeechConfig with a voice from the known pool"""
        voice = TtsVoice((f"model-{uuid.uuid4().hex[:6]}",))
        config = voice.speech()
        name = config.voice_config.prebuilt_voice_config.voice_name
        assert name in TtsVoice._VOICES, "voice name was not from the known pool"

    def test_produces_different_voices_across_calls(self):
        """TtsVoice returns more than one distinct voice over many calls"""
        voice = TtsVoice((f"model-{uuid.uuid4().hex[:6]}",))
        names = {
            voice.speech().voice_config.prebuilt_voice_config.voice_name
            for _ in range(60)
        }
        assert len(names) > 1, "all 60 calls returned the same voice"


class TestVocabularyNoteConvertsNewlinesToLineBreaks:
    """VocabularyNote converts newlines to HTML line breaks in sentence and example"""

    def test_sentence_highlights_word_with_bold_italic(self):
        """VocabularyNote wraps highlight in strong and em tags in sentence field"""
        model = CardModel(StableId(f"Model-{uuid.uuid4().hex[:8]}").value()).model()
        note = VocabularyNote(model)
        word = f"wörd_{uuid.uuid4().hex[:4]}"
        entry = {
            "word": word,
            "pronunciation": "ˈtɛst",
            "translation": "тéст",
            "example": "séntence",
            "sentence": f"Предложéние с {word} внутри",
            "highlight": word,
            "hint": "",
            "context": "",
            "importance": "7",
        }
        result = note.note(entry, "[sound:x.wav]", "<img src='x.jpg'>")
        assert f"<strong><em>{word}</em></strong>" in result.fields[0], "sentence field did not contain highlighted word"

    def test_example_newlines_become_br_tags(self):
        """VocabularyNote replaces newlines with br tags in example field"""
        model = CardModel(StableId(f"Model-{uuid.uuid4().hex[:8]}").value()).model()
        note = VocabularyNote(model)
        delimiter = uuid.uuid4().hex[:6]
        entry = {
            "word": f"λέξη_{uuid.uuid4().hex[:4]}",
            "pronunciation": "ˈleksi",
            "translation": "слóво",
            "example": f"— Γεια σου\n— Γεια {delimiter}",
            "sentence": "Привéт",
            "highlight": "",
            "hint": "",
            "context": "",
            "importance": "8",
        }
        result = note.note(entry, "[sound:x.wav]", "<img src='x.jpg'>")
        assert "<br>" in result.fields[4], "example field did not contain br tag after newline conversion"
