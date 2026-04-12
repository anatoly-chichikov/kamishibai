"""Filesystem location resolution for kamishibai."""

import os
import sys
from pathlib import Path
from typing import final


def cache_home():
    """Return the platform-appropriate user cache directory."""
    if sys.platform == "darwin":
        return (Path.home() / "Library" / "Caches").resolve()
    value = os.environ.get("XDG_CACHE_HOME")
    if value:
        return Path(value).expanduser().resolve()
    return (Path.home() / ".cache").resolve()


def data_home():
    """Return the platform-appropriate user data directory."""
    if sys.platform == "darwin":
        return (Path.home() / "Library" / "Application Support").resolve()
    value = os.environ.get("XDG_DATA_HOME")
    if value:
        return Path(value).expanduser().resolve()
    return (Path.home() / ".local" / "share").resolve()


def cache_root():
    """Return the default cache root for kamishibai artifacts."""
    value = os.environ.get("KAMISHIBAI_CACHE")
    if value:
        return Path(value).expanduser().resolve()
    return cache_home() / "kamishibai"


@final
class Locations:
    """Resolves input, output, and cache paths for a CLI invocation."""

    def __init__(self, args):
        self._args = args

    def input(self):
        """Return absolute path to the input JSON document."""
        value = self._args.path or os.environ.get("KAMISHIBAI_INPUT")
        if value:
            return Path(value).expanduser().resolve()
        default = Path.cwd() / "kamishibai.json"
        if default.is_file():
            return default.resolve()
        raise ValueError(
            "Input JSON path is not set; pass a path, set KAMISHIBAI_INPUT, or place kamishibai.json in the current directory"
        )

    def output(self):
        """Return directory that stores generated output artifacts."""
        value = getattr(self._args, "output", None) or os.environ.get("KAMISHIBAI_OUTPUT")
        if value:
            return Path(value).expanduser().resolve()
        return self.input().parent / "output"

    def cache(self):
        """Return directory that stores reusable media cache."""
        value = getattr(self._args, "cache", None)
        if value:
            return Path(value).expanduser().resolve()
        return cache_root()
