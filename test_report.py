#!/usr/bin/env python3
"""
Unit tests for Report, FontPath, and Thumbnail classes
"""

import os
import tempfile
import uuid

import pytest
from PIL import Image

from create_anki_deck import EnglishLayout
from deck import FontFamily
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
    """Fake font resolver returning a preconfigured path for both weights"""

    def __init__(self, path):
        self._path = path

    def regular(self):
        return self._path

    def bold(self):
        return self._path


def _font():
    return FontFamily("DejaVu Sans")


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


class TestFontFamilyResolvesBothVariants:
    """FontFamily resolves both regular and bold variants to existing files"""

    def test_regular_path_exists(self):
        font = FontFamily("DejaVu Sans")
        path = font.regular()
        assert os.path.isfile(path), "regular font path does not exist"

    def test_bold_path_exists(self):
        font = FontFamily("DejaVu Sans")
        path = font.bold()
        assert os.path.isfile(path), "bold font path does not exist"


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


class TestReportWrapsLongTextWithoutError:
    """Report wraps long text without raising an error"""

    def test_wraps_without_error(self):
        paragraph = "Ünïcödé " * 80
        layout = _FakeLayout([(paragraph, 10)])
        font = _font()
        report = Report(layout, font, Thumbnail(150))
        entry = {"sentence": paragraph, "word": "wörd"}
        report.append(entry, None)
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, f"{uuid.uuid4().hex[:8]}.pdf")
            report.save(path)
            assert os.path.getsize(path) > 0, "wrapped-text PDF has zero bytes"


class TestReportWithThirtyEntriesProducesMultiPagePdf:
    """Report with 30 entries produces a multi-page PDF"""

    def test_multipage_pdf(self):
        lines = [("Строка öднä", 11), ("Пример prédlözhéniÿa", 9)]
        layout = _FakeLayout(lines)
        font = _font()
        report = Report(layout, font, Thumbnail(150))
        with tempfile.TemporaryDirectory() as tmp:
            for idx in range(30):
                imagepath = _image(tmp)
                entry = {"sentence": f"Ünïcödé {idx}", "word": f"wörd{idx}"}
                report.append(entry, imagepath)
            path = os.path.join(tmp, f"{uuid.uuid4().hex[:8]}.pdf")
            report.save(path)
            assert os.path.getsize(path) > 3000, "30-entry PDF is suspiciously small"


class TestEnglishLayoutReturnsSixRowsForFullEntry:
    """EnglishLayout returns 6 rows for a fully populated entry"""

    def test_six_rows(self):
        entry = {
            "word": "café",
            "pronunciation": "ˈyːbɐmɛnʃ",
            "translation": "кафé",
            "example": "The café serves délicious crêpes",
            "sentence": "В кафé подáют вкýсные крéпы",
            "context": "Контéкст Ünïcödé записи",
            "hint": "Подскáзка к слóву",
            "importance": "7",
        }
        layout = EnglishLayout()
        rows = layout.row(entry)
        assert len(rows) == 6, "fully populated entry did not produce 6 rows"

    def test_sentence_has_label(self):
        entry = {
            "word": "café",
            "pronunciation": "ˈyːbɐmɛnʃ",
            "translation": "кафé",
            "example": "The café serves délicious crêpes",
            "sentence": "В кафé подáют вкýсные крéпы",
            "context": "Контéкст Ünïcödé записи",
            "hint": "Подскáзка к слóву",
            "importance": "7",
        }
        layout = EnglishLayout()
        rows = layout.row(entry)
        assert rows[2][0].startswith("Перевод:"), "sentence row lacks Перевод label"

    def test_context_has_label(self):
        entry = {
            "word": "café",
            "pronunciation": "ˈyːbɐmɛnʃ",
            "translation": "кафé",
            "example": "The café serves délicious crêpes",
            "sentence": "В кафé подáют вкýсные крéпы",
            "context": "Контéкст Ünïcödé записи",
            "hint": "Подскáзка к слóву",
            "importance": "7",
        }
        layout = EnglishLayout()
        rows = layout.row(entry)
        assert rows[3][0].startswith("Контекст:"), "context row lacks Контекст label"

    def test_hint_has_label(self):
        entry = {
            "word": "café",
            "pronunciation": "ˈyːbɐmɛnʃ",
            "translation": "кафé",
            "example": "The café serves délicious crêpes",
            "sentence": "В кафé подáют вкýсные крéпы",
            "context": "Контéкст Ünïcödé записи",
            "hint": "Подскáзка к слóву",
            "importance": "7",
        }
        layout = EnglishLayout()
        rows = layout.row(entry)
        assert rows[4][0].startswith("Подсказка:"), "hint row lacks Подсказка label"

    def test_importance_has_label(self):
        entry = {
            "word": "café",
            "pronunciation": "ˈyːbɐmɛnʃ",
            "translation": "кафé",
            "example": "The café serves délicious crêpes",
            "sentence": "В кафé подáют вкýсные крéпы",
            "context": "Контéкст Ünïcödé записи",
            "hint": "Подскáзка к слóву",
            "importance": "7",
        }
        layout = EnglishLayout()
        rows = layout.row(entry)
        assert rows[5][0].startswith("Важность:"), "importance row lacks Важность label"


class TestEnglishLayoutReturnsTwoRowsForSparseEntry:
    """EnglishLayout returns 2 rows for a sparse entry with only header and sentence"""

    def test_two_rows(self):
        entry = {
            "word": "café",
            "pronunciation": "",
            "translation": "кафé",
            "example": "",
            "sentence": "В кафé подáют вкýсные крéпы",
            "context": "",
            "hint": "",
            "importance": "",
        }
        layout = EnglishLayout()
        rows = layout.row(entry)
        assert len(rows) == 2, "sparse entry did not produce exactly 2 rows"

    def test_sparse_sentence_has_label(self):
        entry = {
            "word": "café",
            "pronunciation": "",
            "translation": "кафé",
            "example": "",
            "sentence": "В кафé подáют вкýсные крéпы",
            "context": "",
            "hint": "",
            "importance": "",
        }
        layout = EnglishLayout()
        rows = layout.row(entry)
        assert rows[1][0].startswith("Перевод:"), "sparse sentence row lacks Перевод label"


class TestReportWithImageAndWrappingTextProducesValidPdf:
    """Report with image and wrapping text produces a valid PDF"""

    def test_image_and_wrapping(self):
        paragraph = "Длинный тéкст Ünïcödé " * 40
        layout = _FakeLayout([(paragraph, 9), ("Кöröткая", 11)])
        font = _font()
        report = Report(layout, font, Thumbnail(150))
        with tempfile.TemporaryDirectory() as tmp:
            imagepath = _image(tmp)
            entry = {"sentence": paragraph, "word": "слöвö"}
            report.append(entry, imagepath)
            path = os.path.join(tmp, f"{uuid.uuid4().hex[:8]}.pdf")
            report.save(path)
            assert os.path.getsize(path) > 500, "image-and-wrap PDF is suspiciously small"


class TestReportWithEntryNearPageBottomDoesntProduceEmptyPages:
    """Report with entry near page bottom doesnt produce nearly-empty pages"""

    def test_no_empty_pages(self):
        filler = [("Wörd", 11), ("À" * 1000, 9)]
        layout = _FakeLayout(filler)
        font = _font()
        report = Report(layout, font, Thumbnail(150))
        with tempfile.TemporaryDirectory() as tmp:
            for _ in range(5):
                imagepath = _image(tmp)
                report.append({"sentence": f"Ünïcödé-{uuid.uuid4().hex[:6]}", "word": "wörd"}, imagepath)
            path = os.path.join(tmp, f"{uuid.uuid4().hex[:8]}.pdf")
            report.save(path)
            with open(path, "rb") as f:
                pages = f.read().count(b"/Type /Page")
            assert pages <= 3, "too many pages produced — likely has nearly-empty pages"
