#!/usr/bin/env python3
"""Unit tests for filesystem location resolution."""

import tempfile
import uuid
from pathlib import Path

import pytest

from kamishibai import locations
from kamishibai.locations import Locations


class _Args:
    """Simple namespace for location tests."""

    def __init__(self, path=None, output=None, cache=None):
        self.path = path
        self.output = output
        self.cache = cache


class TestLocationsResolveInput:
    """Locations resolves input documents from explicit configuration."""

    def test_uses_explicit_input_path(self):
        directory = Path(tempfile.mkdtemp())
        path = directory / f"λέξη_{uuid.uuid4().hex[:4]}.json"
        result = Locations(_Args(path=str(path))).input()
        assert result == path.resolve(), \
            "explicit input path was not resolved"

    def test_uses_environment_input_path(self, monkeypatch):
        directory = Path(tempfile.mkdtemp())
        path = directory / f"слово_{uuid.uuid4().hex[:4]}.json"
        monkeypatch.setenv("KAMISHIBAI_INPUT", str(path))
        result = Locations(_Args()).input()
        assert result == path.resolve(), \
            "environment input path was not resolved"

    def test_uses_kamishibai_json_from_current_directory(self, monkeypatch):
        directory = Path(tempfile.mkdtemp())
        path = directory / "kamishibai.json"
        path.write_text("{}", encoding="utf-8")
        monkeypatch.chdir(directory)
        monkeypatch.delenv("KAMISHIBAI_INPUT", raising=False)
        result = Locations(_Args()).input()
        assert result == path.resolve(), \
            "current-directory kamishibai.json was not resolved"

    def test_rejects_missing_input_path(self, monkeypatch):
        monkeypatch.delenv("KAMISHIBAI_INPUT", raising=False)
        monkeypatch.chdir(Path(tempfile.mkdtemp()))
        rejected = False
        try:
            Locations(_Args()).input()
        except ValueError:
            rejected = True
        assert rejected, "missing input path was not rejected"


class TestLocationsResolveOutput:
    """Locations resolves output directories from args or input placement."""

    def test_uses_explicit_output_path(self):
        directory = Path(tempfile.mkdtemp())
        output = directory / f"вывод_{uuid.uuid4().hex[:4]}"
        result = Locations(_Args(path="/tmp/input.json", output=str(output))).output()
        assert result == output.resolve(), \
            "explicit output path was not resolved"

    def test_uses_environment_output_path(self, monkeypatch):
        directory = Path(tempfile.mkdtemp())
        output = directory / f"έξοδος_{uuid.uuid4().hex[:4]}"
        monkeypatch.setenv("KAMISHIBAI_OUTPUT", str(output))
        result = Locations(_Args(path="/tmp/input.json")).output()
        assert result == output.resolve(), \
            "environment output path was not resolved"

    def test_places_default_output_beside_input(self):
        directory = Path(tempfile.mkdtemp()) / f"данные_{uuid.uuid4().hex[:4]}"
        result = Locations(_Args(path=str(directory / "kamishibai.json"))).output()
        assert result == (directory / "output").resolve(), \
            "default output directory was not placed beside the input"


class TestLocationsResolveCache:
    """Locations resolves cache directories without depending on cwd."""

    def test_uses_explicit_cache_path(self):
        directory = Path(tempfile.mkdtemp())
        cache = directory / f"кэш_{uuid.uuid4().hex[:4]}"
        result = Locations(_Args(path="/tmp/input.json", cache=str(cache))).cache()
        assert result == cache.resolve(), \
            "explicit cache path was not resolved"

    def test_uses_environment_cache_path(self, monkeypatch):
        directory = Path(tempfile.mkdtemp())
        cache = directory / f"μνήμη_{uuid.uuid4().hex[:4]}"
        monkeypatch.setenv("KAMISHIBAI_CACHE", str(cache))
        result = Locations(_Args(path="/tmp/input.json")).cache()
        assert result == cache.resolve(), \
            "environment cache path was not resolved"

    def test_uses_xdg_cache_root_on_non_darwin(self, monkeypatch):
        directory = Path(tempfile.mkdtemp())
        monkeypatch.setattr(locations.sys, "platform", "linux")
        monkeypatch.setenv("XDG_CACHE_HOME", str(directory))
        result = Locations(_Args(path="/tmp/input.json")).cache()
        assert result == (directory / "kamishibai").resolve(), \
            "xdg cache root was not used on non-darwin platforms"
