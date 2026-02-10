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

from deck import Illustration

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
