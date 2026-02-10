#!/usr/bin/env python3
"""
Unit tests for Report, FontPath, and Thumbnail classes
"""

import os
import tempfile
import uuid

import pytest
from PIL import Image

from deck import FontPath
from deck import Report
from deck import Thumbnail


class _FakeLayout:
    """Fake layout returning fixed text lines"""

    def __init__(self, lines):
        self._lines = lines

    def row(self, entry):
        return self._lines


class _FakeFont:
    """Fake font resolver returning a preconfigured path"""

    def __init__(self, path):
        self._path = path

    def resolved(self):
        return self._path


def _font():
    return FontPath("DejaVu Sans")


def _image(directory, size=128):
    path = os.path.join(directory, f"{uuid.uuid4().hex[:8]}.png")
    img = Image.new("RGB", (size, size), color=(42, 99, 200))
    img.save(path, "PNG")
    return path


class TestReportWithNoEntriesProducesValidPdf:
    """Report with no entries produces a valid PDF file"""

    def test_creates_nonempty_file(self):
        layout = _FakeLayout([("línea única", 10)])
        font = _font()
        report = Report(layout, font, Thumbnail(150))
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, f"{uuid.uuid4().hex[:8]}.pdf")
            report.save(path)
            assert os.path.getsize(path) > 0, "empty report PDF has zero bytes"


class TestReportWithEntryAndImageProducesPdf:
    """Report with one entry and image produces a larger PDF"""

    def test_includes_image_content(self):
        layout = _FakeLayout([("Ünïcödé línë", 10), ("wörd", 14)])
        font = _font()
        report = Report(layout, font, Thumbnail(150))
        with tempfile.TemporaryDirectory() as tmp:
            imagepath = _image(tmp)
            entry = {"sentence": "Ünïcödé", "word": "wörd"}
            report.append(entry, imagepath)
            path = os.path.join(tmp, f"{uuid.uuid4().hex[:8]}.pdf")
            report.save(path)
            size = os.path.getsize(path)
            assert size > 500, "PDF with image is suspiciously small"


class TestReportRendersCyrillicAndGreekText:
    """Report correctly renders Cyrillic and Greek text without errors"""

    def test_renders_multiscript_text(self):
        lines = [
            ("Кирилица проверка", 10),
            ("Ελληνικά δοκιμή", 14),
            ("Mixed Ünïcödé ñ ü ö", 9),
        ]
        layout = _FakeLayout(lines)
        font = _font()
        report = Report(layout, font, Thumbnail(150))
        entry = {"sentence": "Кирилица", "word": "Ελληνικά"}
        with tempfile.TemporaryDirectory() as tmp:
            report.append(entry, None)
            path = os.path.join(tmp, f"{uuid.uuid4().hex[:8]}.pdf")
            report.save(path)
            assert os.path.getsize(path) > 0, "multiscript PDF has zero bytes"


class TestFontPathResolvesDejaVuSans:
    """FontPath resolves DejaVu Sans to an existing TTF file"""

    def test_resolved_path_exists(self):
        font = FontPath("DejaVu Sans")
        path = font.resolved()
        assert os.path.isfile(path), "resolved font path does not exist"


class TestThumbnailCompressesPdfImage:
    """Report with thumbnail produces a smaller PDF than without compression"""

    def test_compressed_pdf_is_smaller(self):
        layout = _FakeLayout([("Ünïcödé línë", 10)])
        font = _font()
        with tempfile.TemporaryDirectory() as tmp:
            imagepath = _image(tmp, size=1008)
            entry = {"sentence": "Ünïcödé", "word": "wörd"}
            compressed = Report(layout, font, Thumbnail(150))
            compressed.append(entry, imagepath)
            small = os.path.join(tmp, f"{uuid.uuid4().hex[:8]}.pdf")
            compressed.save(small)
            uncompressed = Report(layout, font, Thumbnail(1008))
            uncompressed.append(entry, imagepath)
            large = os.path.join(tmp, f"{uuid.uuid4().hex[:8]}.pdf")
            uncompressed.save(large)
            assert os.path.getsize(small) < os.path.getsize(large), \
                "compressed PDF is not smaller than uncompressed"
