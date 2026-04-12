"""PDF reporting for kamishibai."""

import os
import subprocess
import tempfile
from typing import Protocol, final

from fpdf import FPDF
from PIL import Image


class ReportLayout(Protocol):
    """Protocol for formatting entry text lines in a PDF report."""

    def row(self, entry):
        """Return list of (text, font_size) tuples for a single report entry."""
        ...


@final
class VocabularyLayout:
    """Formats vocabulary entries as text lines for PDF report."""

    def row(self, entry):
        """Return list of (text, font_size) tuples for a vocabulary entry."""
        pronunciation = entry.get("pronunciation", "")
        header = entry["word"]
        if pronunciation:
            header += f" /{pronunciation.strip('/')}/"
        header += f' — {entry["translation"]}'
        lines = [(header, 11)]
        example = entry.get("example", "")
        if example:
            lines.append((example, 9))
        sentence = entry.get("sentence", "")
        if sentence:
            lines.append((f"Перевод: {sentence}", 9))
        context = entry.get("context", "")
        if context:
            lines.append((f"Контекст: {context}", 8))
        hint = entry.get("hint", "")
        if hint:
            lines.append((f"Подсказка: {hint}", 8))
        importance = entry.get("importance", "")
        if importance:
            lines.append((f"Важность: {importance}/10", 8))
        return lines


@final
class FontPath:
    """Resolves a font family name to a filesystem path via fc-match."""

    def __init__(self, family):
        self._family = family

    def resolved(self):
        """Return absolute path to the TTF file for the configured family."""
        result = subprocess.run(
            ["fc-match", "-f", "%{file}", self._family],
            capture_output=True,
            text=True,
        )
        path = result.stdout.strip()
        if not path or not os.path.isfile(path):
            raise FileNotFoundError(
                f"Font '{self._family}' not found via fc-match"
            )
        return path


@final
class FontFamily:
    """Resolves regular and bold variants of a font family via fc-match."""

    def __init__(self, family):
        self._regular = FontPath(family)
        self._bold = FontPath(f"{family}:Bold")

    def regular(self):
        """Return absolute path to the regular weight TTF file."""
        return self._regular.resolved()

    def bold(self):
        """Return absolute path to the bold weight TTF file."""
        return self._bold.resolved()


@final
class Thumbnail:
    """Resizes an image to a target pixel size for PDF embedding."""

    def __init__(self, pixels):
        self._pixels = pixels

    def compressed(self, source, directory):
        """Return path to a resized JPEG copy in the given directory."""
        image = Image.open(source)
        image.thumbnail((self._pixels, self._pixels))
        name = f"thumb_{os.path.basename(source)}"
        result = os.path.join(directory, name)
        image.save(result, "JPEG", quality=60)
        return result


@final
class Report:
    """Accumulates vocabulary entries and renders a PDF report."""

    def __init__(self, layout, font, thumbnail):
        self._layout = layout
        self._font = font
        self._thumbnail = thumbnail
        self._rows = []
        self._fonts = {}

    def append(self, entry, imagepath):
        """Record an entry with its image path for later rendering."""
        self._rows.append((entry, imagepath))

    def save(self, output):
        """Render all accumulated entries to a PDF file."""
        pdf = FPDF()
        pdf.set_auto_page_break(auto=True, margin=15)
        alias = self._alias(pdf, {})
        pdf.set_font(alias, size=10)
        pdf.add_page()
        with tempfile.TemporaryDirectory() as thumbdir:
            for entry, imagepath in self._rows:
                if pdf.get_y() > 240:
                    pdf.add_page()
                self._row(pdf, entry, imagepath, thumbdir)
        pdf.output(output)

    def _alias(self, pdf, entry):
        """Return a registered PDF font alias for the given entry."""
        font = self._font.selected(entry) if hasattr(self._font, "selected") else self._font
        regular = font.regular()
        bold = font.bold()
        key = (regular, bold)
        if key not in self._fonts:
            alias = f"font{len(self._fonts)}"
            pdf.add_font(alias, "", regular)
            pdf.add_font(alias, "B", bold)
            self._fonts[key] = alias
        return self._fonts[key]

    def _row(self, pdf, entry, imagepath, thumbdir):
        """Render a single entry row with optional image thumbnail."""
        top = pdf.get_y()
        page = pdf.page
        alias = self._alias(pdf, entry)
        if imagepath and os.path.isfile(imagepath):
            thumb = self._thumbnail.compressed(imagepath, thumbdir)
            pdf.image(thumb, x=10, y=top, w=25, h=25)
        indent = 40
        width = pdf.w - indent - pdf.r_margin
        pdf.set_xy(indent, top)
        for idx, (text, size) in enumerate(self._layout.row(entry)):
            if idx == 0:
                pdf.set_font(alias, style="B", size=size)
                pdf.set_text_color(0, 0, 0)
            elif size <= 8:
                pdf.set_font(alias, style="", size=size)
                pdf.set_text_color(120, 120, 120)
            else:
                pdf.set_font(alias, style="", size=size)
                pdf.set_text_color(0, 0, 0)
            pdf.set_x(indent)
            pdf.multi_cell(w=width, h=size * 0.5, text=str(text), align="L")
        if pdf.page != page:
            top = pdf.t_margin
        bottom = max(pdf.get_y(), top + 25)
        pdf.set_y(bottom)
        pdf.ln(3)
        pdf.set_draw_color(200, 200, 200)
        pdf.line(10, pdf.get_y(), pdf.w - pdf.r_margin, pdf.get_y())
        pdf.ln(4)
