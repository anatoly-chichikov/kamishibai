#!/usr/bin/env python3
"""
Tests for progress display classes
"""

import logging
import os
import tempfile
import uuid

import pytest

from progress import PlainProgress, ProgressSelector, RichProgress

logging.disable(logging.CRITICAL)


@pytest.fixture()
def collector():
    """Return a fresh list to collect output lines"""
    return []


class FakeConsole:
    """Fake rich Console that records print calls"""

    def __init__(self, lines):
        self._lines = lines
        self._kwargs = []

    def print(self, text, **kwargs):
        """Record printed text and keyword arguments"""
        self._lines.append(text)
        self._kwargs.append(kwargs)

    def status(self, text, spinner="dots"):
        """Return a FakeSpinner"""
        return FakeSpinner(self._lines)


class FakeSpinner:
    """Fake rich status spinner that records start/stop"""

    def __init__(self, lines):
        self._lines = lines

    def update(self, text):
        """Record update"""
        self._lines.append(f"spinner:{text}")

    def start(self):
        """Record start"""
        self._lines.append("spinner:start")

    def stop(self):
        """Record stop"""
        self._lines.append("spinner:stop")


class TestPlainProgress:
    """Tests for PlainProgress"""

    def test_card_prints_header_with_nonascii_word(self, collector):
        """PlainProgress prints card header with non-ASCII word"""
        progress = PlainProgress(collector.append)
        word = f"\u00fcber-{uuid.uuid4().hex[:4]}"
        progress.card(3, 10, word)
        assert f"Processing card 3/10: {word}" in collector[0], \
            "card header did not contain expected word"

    def test_step_produces_no_output(self, collector):
        """PlainProgress step is silent"""
        progress = PlainProgress(collector.append)
        progress.step(f"step-{uuid.uuid4().hex[:4]}")
        assert len(collector) == 0, \
            "step should not produce output in plain mode"

    def test_done_prints_name_and_label(self, collector):
        """PlainProgress done prints step name and label"""
        progress = PlainProgress(collector.append)
        name = f"G\u00e9n\u00e9ration-{uuid.uuid4().hex[:4]}"
        label = f"r\u00e9sultat-{uuid.uuid4().hex[:4]}"
        progress.done(name, label)
        assert f"  {name}: {label}" == collector[0], \
            "done output did not match expected format"

    def test_done_appends_basename_when_path_given(self, collector):
        """PlainProgress done appends basename in parentheses when path provided"""
        progress = PlainProgress(collector.append)
        directory = tempfile.mkdtemp()
        filename = f"{uuid.uuid4().hex[:12]}.wav"
        path = os.path.join(directory, filename)
        progress.done("Generating audio", "generated", path)
        assert f"({filename})" in collector[0], \
            "done did not append basename for given path"

    def test_done_omits_path_when_empty(self, collector):
        """PlainProgress done omits path suffix when path is empty string"""
        progress = PlainProgress(collector.append)
        name = f"step-{uuid.uuid4().hex[:4]}"
        progress.done(name, "cached", "")
        assert "(" not in collector[0], \
            "done showed parentheses despite empty path"

    def test_retry_prints_reason_and_attempt(self, collector):
        """PlainProgress retry includes reason and attempt number"""
        progress = PlainProgress(collector.append)
        reason = f"\u041e\u0448\u0438\u0431\u043a\u0430-{uuid.uuid4().hex[:4]}"
        progress.retry("Rendering", 2, reason)
        assert "(attempt 2)" in collector[0], \
            "retry did not include attempt number"

    def test_skip_prints_word_and_reason(self, collector):
        """PlainProgress skip includes word and reason"""
        progress = PlainProgress(collector.append)
        word = f"w\u00f6rd-{uuid.uuid4().hex[:4]}"
        reason = f"err-{uuid.uuid4().hex[:4]}"
        progress.skip(word, reason)
        assert word in collector[0] and reason in collector[0], \
            "skip did not include word and reason"

    def test_finish_prints_summary(self, collector):
        """PlainProgress finish prints processed count"""
        progress = PlainProgress(collector.append)
        progress.finish(7, 10, [])
        assert "7/10" in collector[0], \
            "finish did not include card counts"

    def test_finish_prints_failures(self, collector):
        """PlainProgress finish lists failed cards"""
        progress = PlainProgress(collector.append)
        word = f"fail-{uuid.uuid4().hex[:4]}"
        failures = [{"word": word, "reason": "timeout"}]
        progress.finish(9, 10, failures)
        combined = " ".join(collector)
        assert word in combined, \
            "finish did not list failed word"

    def test_result_prints_basename_and_full_path(self, collector):
        """PlainProgress result shows basename followed by full path"""
        progress = PlainProgress(collector.append)
        directory = tempfile.mkdtemp()
        filename = f"cards_{uuid.uuid4().hex[:6]}.apkg"
        path = os.path.join(directory, filename)
        progress.result("Anki deck", path)
        assert filename in collector[0], \
            "result did not include basename"
        assert path in collector[0], \
            "result did not include full path"


class TestRichProgress:
    """Tests for RichProgress"""

    def test_card_prints_bold_header(self, collector):
        """RichProgress card emits bold markup"""
        console = FakeConsole(collector)
        spinner = FakeSpinner(collector)
        progress = RichProgress(console, spinner)
        word = f"r\u00e9silience-{uuid.uuid4().hex[:4]}"
        progress.card(3, 10, word)
        assert word in collector[0], \
            "card did not contain word"

    def test_step_starts_spinner(self, collector):
        """RichProgress step starts the spinner"""
        console = FakeConsole(collector)
        spinner = FakeSpinner(collector)
        progress = RichProgress(console, spinner)
        progress.step(f"step-{uuid.uuid4().hex[:4]}")
        assert "spinner:start" in collector, \
            "step did not start spinner"

    def test_done_stops_spinner_and_prints_checkmark(self, collector):
        """RichProgress done stops spinner and emits checkmark"""
        console = FakeConsole(collector)
        spinner = FakeSpinner(collector)
        progress = RichProgress(console, spinner)
        progress.done("Audio", f"lbl-{uuid.uuid4().hex[:4]}")
        assert "spinner:stop" in collector, \
            "done did not stop spinner"
        printed = [x for x in collector if "\u2714" in x]
        assert len(printed) == 1, \
            "done did not emit checkmark"

    def test_done_includes_link_markup_when_path_given(self, collector):
        """RichProgress done includes rich link markup for file path"""
        console = FakeConsole(collector)
        spinner = FakeSpinner(collector)
        progress = RichProgress(console, spinner)
        directory = tempfile.mkdtemp()
        filename = f"{uuid.uuid4().hex[:12]}.json"
        path = os.path.join(directory, filename)
        progress.done("Composing scene", "translated", path)
        printed = [x for x in collector if "\u2714" in x]
        assert f"[link=file://{path}]" in printed[0], \
            "done did not include rich link markup"
        assert filename in printed[0], \
            "done did not include basename in link text"

    def test_done_omits_link_when_path_empty(self, collector):
        """RichProgress done omits link markup when path is empty"""
        console = FakeConsole(collector)
        spinner = FakeSpinner(collector)
        progress = RichProgress(console, spinner)
        progress.done("Composing scene", "cached", "")
        printed = [x for x in collector if "\u2714" in x]
        assert "[link=" not in printed[0], \
            "done showed link markup despite empty path"

    def test_retry_restarts_spinner(self, collector):
        """RichProgress retry stops then restarts spinner"""
        console = FakeConsole(collector)
        spinner = FakeSpinner(collector)
        progress = RichProgress(console, spinner)
        progress.retry("Rendering", 1, f"reason-{uuid.uuid4().hex[:4]}")
        assert collector.count("spinner:stop") == 1, \
            "retry did not stop spinner"
        assert collector.count("spinner:start") == 1, \
            "retry did not restart spinner"

    def test_skip_stops_spinner_and_prints_cross(self, collector):
        """RichProgress skip stops spinner and emits cross mark"""
        console = FakeConsole(collector)
        spinner = FakeSpinner(collector)
        progress = RichProgress(console, spinner)
        word = f"sk\u00edp-{uuid.uuid4().hex[:4]}"
        progress.skip(word, "error")
        assert "spinner:stop" in collector, \
            "skip did not stop spinner"
        printed = [x for x in collector if "\u2718" in x]
        assert len(printed) == 1, \
            "skip did not emit cross mark"

    def test_finish_prints_bold_summary(self, collector):
        """RichProgress finish emits bold summary"""
        console = FakeConsole(collector)
        spinner = FakeSpinner(collector)
        progress = RichProgress(console, spinner)
        progress.finish(5, 8, [])
        assert "5/8" in collector[0], \
            "finish did not include counts"

    def test_result_includes_link_markup(self, collector):
        """RichProgress result shows checkmark with clickable link"""
        console = FakeConsole(collector)
        spinner = FakeSpinner(collector)
        progress = RichProgress(console, spinner)
        directory = tempfile.mkdtemp()
        filename = f"greek_{uuid.uuid4().hex[:6]}.apkg"
        path = os.path.join(directory, filename)
        progress.result("Anki deck", path)
        assert f"[link=file://{path}]" in collector[0], \
            "result did not include rich link markup"
        assert filename in collector[0], \
            "result did not include basename"


class TestRichProgressDoneDisablesHighlighting:
    """RichProgress done disables highlight to prevent number colorization in paths"""

    def test_highlight_disabled(self, collector):
        console = FakeConsole(collector)
        spinner = FakeSpinner(collector)
        progress = RichProgress(console, spinner)
        path = f"/tmp/{uuid.uuid4().hex[:6]}/greek_2026-02-10.json"
        progress.done("Composing scene", "cached", path)
        assert console._kwargs[-1].get("highlight") is False, \
            "done did not disable highlight for path output"


class TestRichProgressResultDisablesHighlighting:
    """RichProgress result disables highlight to prevent number colorization in paths"""

    def test_highlight_disabled(self, collector):
        console = FakeConsole(collector)
        spinner = FakeSpinner(collector)
        progress = RichProgress(console, spinner)
        path = f"/tmp/{uuid.uuid4().hex[:6]}/greek_2026-02-10.apkg"
        progress.result("Anki deck", path)
        assert console._kwargs[-1].get("highlight") is False, \
            "result did not disable highlight for path output"


class TestProgressSelector:
    """Tests for ProgressSelector"""

    def test_selects_plain_for_noninteractive(self):
        """ProgressSelector returns PlainProgress when terminal is False"""
        result = ProgressSelector(False).selected()
        assert isinstance(result, PlainProgress), \
            "non-interactive terminal did not select PlainProgress"

    def test_selects_rich_for_interactive(self):
        """ProgressSelector returns RichProgress when terminal is True"""
        result = ProgressSelector(True).selected()
        assert isinstance(result, RichProgress), \
            "interactive terminal did not select RichProgress"
