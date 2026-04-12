#!/usr/bin/env python3
"""
Tests for diagnosis error display classes
"""

import json
import logging
import os
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

import pytest

from kamishibai.diagnosis import DiagnosisSelector
from kamishibai.diagnosis import PlainDiagnosis
from kamishibai.diagnosis import RichDiagnosis

logging.disable(logging.CRITICAL)

ROOT = Path(__file__).resolve().parents[1]


class TestPlainDiagnosisShowsErrorPrefixAndMessage:
    """PlainDiagnosis prints error prefix followed by the message"""

    def test_shows_error_prefix_and_message(self):
        """PlainDiagnosis prints 'Error: {message}' to the output"""
        lines = []
        diagnosis = PlainDiagnosis(lines.append)
        message = f"Файл не найден {uuid.uuid4().hex[:6]}"
        diagnosis.show(message, "")
        assert f"Error: {message}" == lines[0], \
            "plain diagnosis did not show error prefix with message"


class TestPlainDiagnosisShowsPathWhenProvided:
    """PlainDiagnosis includes file path when one is given"""

    def test_shows_path_when_provided(self):
        """PlainDiagnosis appends file path on a separate line"""
        lines = []
        diagnosis = PlainDiagnosis(lines.append)
        path = f"/tmp/{uuid.uuid4().hex[:8]}/вокабуляр.json"
        diagnosis.show("Something broke", path)
        combined = "\n".join(lines)
        assert path in combined, \
            "plain diagnosis did not include file path in output"


class TestRichDiagnosisShowsPanelWithMessage:
    """RichDiagnosis renders a rich panel containing the error message"""

    def test_shows_panel_with_message(self):
        """RichDiagnosis prints a panel that contains the error text"""
        printed = []

        class FakeConsole:
            """Records print calls"""

            def print(self, renderable, **kwargs):
                """Record the renderable object"""
                printed.append(renderable)

        console = FakeConsole()
        diagnosis = RichDiagnosis(console)
        message = f"Ключ не задан {uuid.uuid4().hex[:6]}"
        diagnosis.show(message, "")
        assert len(printed) == 1, \
            "rich diagnosis did not print exactly one renderable"


class TestDiagnosisSelectorReturnsPlainForPipe:
    """DiagnosisSelector returns PlainDiagnosis when terminal is False"""

    def test_returns_plain_for_pipe(self):
        """DiagnosisSelector selects PlainDiagnosis for non-interactive stderr"""
        result = DiagnosisSelector(False).selected()
        assert isinstance(result, PlainDiagnosis), \
            "selector did not return PlainDiagnosis for piped output"


class TestDiagnosisSelectorReturnsRichForTerminal:
    """DiagnosisSelector returns RichDiagnosis when terminal is True"""

    def test_returns_rich_for_terminal(self):
        """DiagnosisSelector selects RichDiagnosis for interactive terminal"""
        result = DiagnosisSelector(True).selected()
        assert isinstance(result, RichDiagnosis), \
            "selector did not return RichDiagnosis for interactive terminal"


class TestMainExitsOnMissingApiKey:
    """Application exits with code 1 when GEMINI_API_KEY is not set"""

    def test_exits_on_missing_api_key(self):
        """Running kamishibai without GEMINI_API_KEY exits with code 1"""
        env = {k: v for k, v in os.environ.items() if k != "GEMINI_API_KEY"}
        result = subprocess.run(
            [sys.executable, "-m", "kamishibai", "/tmp/nonexistent.json"],
            capture_output=True,
            text=True,
            env=env,
            cwd=ROOT,
            timeout=10,
        )
        assert result.returncode == 1, \
            "missing GEMINI_API_KEY did not produce exit code 1"


class TestMainExitsOnMissingInputFile:
    """Application exits with code 1 when input file does not exist"""

    def test_exits_on_missing_input_file(self):
        """Running kamishibai with nonexistent path exits with code 1"""
        missing = f"/tmp/{uuid.uuid4().hex}_отсутствует.json"
        env = dict(os.environ, GEMINI_API_KEY="fake-key-for-test")
        result = subprocess.run(
            [sys.executable, "-m", "kamishibai", missing],
            capture_output=True,
            text=True,
            env=env,
            cwd=ROOT,
            timeout=10,
        )
        assert result.returncode == 1, \
            "missing input file did not produce exit code 1"
