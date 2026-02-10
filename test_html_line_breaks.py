#!/usr/bin/env python3
"""
Unit tests for HtmlLineBreaks class
"""

import uuid

from deck import HtmlLineBreaks


class TestHtmlLineBreaksReplacesNewlines:
    """
    HtmlLineBreaks converts newline characters into HTML br tags
    """

    def test_replaces_single_newline_with_br_tag(self):
        text = f"строка_{uuid.uuid4().hex[:6]}\nстрока_{uuid.uuid4().hex[:6]}"
        result = HtmlLineBreaks(text).formatted()
        assert "<br>" in result, "single newline was not replaced with br tag"

    def test_replaces_multiple_newlines_with_br_tags(self):
        fragment = uuid.uuid4().hex[:4]
        text = f"α_{fragment}\nβ_{fragment}\nγ_{fragment}"
        result = HtmlLineBreaks(text).formatted()
        assert result.count("<br>") == 2, "not all newlines were replaced with br tags"

    def test_returns_empty_string_for_empty_input(self):
        result = HtmlLineBreaks("").formatted()
        assert result == "", "empty input did not produce empty output"

    def test_preserves_text_without_newlines(self):
        text = f"κείμενο_{uuid.uuid4().hex[:8]}"
        result = HtmlLineBreaks(text).formatted()
        assert result == text, "text without newlines was altered"

    def test_handles_non_ascii_content(self):
        fragment = uuid.uuid4().hex[:4]
        text = f"日本語_{fragment}\nΕλληνικά_{fragment}"
        result = HtmlLineBreaks(text).formatted()
        assert "\n" not in result, "newline survived in non-ascii content"
