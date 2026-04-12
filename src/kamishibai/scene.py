"""Image generation primitives for kamishibai."""

import json
import os
import re
import tempfile
from io import BytesIO
from typing import final

import numpy as np
import pytesseract
from google.genai import types
from PIL import Image

from .locations import cache_root


@final
class Cache:
    """Persistent file cache for generated media."""

    def __init__(self, name, root=None):
        self._root = root if root is not None else str(cache_root())
        self._path = os.path.join(self._root, name)

    def root(self):
        """Return root cache directory path."""
        return self._root

    def path(self):
        """Return cache directory path."""
        return self._path

    def exists(self, filename):
        """Check if file exists in cache."""
        return os.path.exists(os.path.join(self._path, filename))

    def filepath(self, filename):
        """Return full path to cached file, ensuring directory exists."""
        os.makedirs(self._path, exist_ok=True)
        return os.path.join(self._path, filename)

    def stage(self, suffix):
        """Return a temporary file path in the cache directory for atomic writes."""
        os.makedirs(self._path, exist_ok=True)
        fd, path = tempfile.mkstemp(suffix=suffix, dir=self._path)
        os.close(fd)
        return path

    def commit(self, staged, filename):
        """Atomically move a staged temp file to its final cache path."""
        os.replace(staged, os.path.join(self._path, filename))


@final
class SceneTranslator:
    """Translates a sentence into manga panel JSON via Gemini text model."""

    def __init__(self, client, prompt, template):
        self._client = client
        self._prompt = prompt
        self._template = template

    def translate(self, sentence, target):
        """Translate sentence to manga_panel dict by merging generated panels into a template."""
        filled = self._prompt.format(sentence=sentence)
        response = self._client.models.generate_content(
            model="gemini-3-flash-preview",
            contents=[filled],
        )
        raw = "".join(
            part.text for part in response.candidates[0].content.parts
            if part.text is not None
        )
        cleaned = re.sub(r"^```(?:json)?\s*", "", raw.strip())
        cleaned = re.sub(r"\s*```$", "", cleaned)
        panels = json.loads(cleaned)
        if not isinstance(panels, list):
            raise ValueError("Expected a JSON array of panels")
        result = json.loads(json.dumps(self._template))
        result["manga_panel"]["panels"] = panels
        result["manga_panel"]["meta"]["title"] = sentence[:60]
        result["manga_panel"]["meta"]["description"] = sentence
        result["manga_panel"]["meta"]["target_lang"] = target
        self._enforce(result)
        self._validate(result)
        return result

    def _enforce(self, panel):
        """Clamp panel bounds and enforce text-free rendering constraints."""
        for entry in panel["manga_panel"]["panels"]:
            entry["bleed"] = False
            bounds = entry.get("bounds", {})
            x = max(bounds.get("x", 0), 16)
            y = max(bounds.get("y", 0), 16)
            width = bounds.get("width", 992)
            height = bounds.get("height", 992)
            bounds["x"] = x
            bounds["y"] = y
            bounds["width"] = min(width, 1008 - x)
            bounds["height"] = min(height, 1008 - y)
            entry["bounds"] = bounds
            scene = entry.get("scene") or entry.get("description")
            if isinstance(scene, dict):
                scene["text_in_frame"] = "none"
            elif isinstance(scene, str):
                entry["text_in_frame"] = "none"

    def _validate(self, panel):
        """Validate minimal structural requirements."""
        root = panel["manga_panel"]
        panels = root.get("panels", [])
        if not panels:
            raise ValueError("No panels found in scene JSON")


@final
class TextDetector:
    """Detects text in images using Tesseract OCR."""

    def __init__(self, threshold, lang="eng"):
        self._threshold = threshold
        self._lang = self._resolved(lang)

    def _resolved(self, lang):
        """Return only installed Tesseract languages from a plus-separated string."""
        available = pytesseract.get_languages()
        requested = lang.split("+")
        supported = [code for code in requested if code in available]
        if not supported:
            return "eng"
        return "+".join(supported)

    def detected(self, image):
        """Return detected text if confidence exceeds threshold, else empty string."""
        data = pytesseract.image_to_data(image, lang=self._lang, output_type=pytesseract.Output.DICT)
        words = []
        for index, text in enumerate(data["text"]):
            stripped = text.strip()
            if len(stripped) >= 2 and int(data["conf"][index]) > self._threshold:
                words.append(stripped)
        if words:
            return " ".join(words)
        return ""


@final
class TextDetectors:
    """Selects a TextDetector from scene metadata and delegates OCR."""

    def __init__(self, detectors, fallback):
        self._detectors = detectors
        self._fallback = fallback

    def detected(self, scene, image):
        """Return detected text using the detector selected by scene target language."""
        root = scene.get("manga_panel", {})
        meta = root.get("meta", {})
        code = meta.get("target_lang", "")
        detector = self._detectors.get(code, self._fallback)
        return detector.detected(image)


@final
class BorderDetector:
    """Detects outer borders and horizontal gutters between manga panels."""

    def __init__(self, width, brightness, margin):
        self._width = width
        self._brightness = brightness
        self._margin = margin

    def gutter(self, image):
        """Return True if at least one white horizontal gutter exists."""
        pixels = np.array(image)
        rows = np.mean(pixels, axis=1)
        white = rows > self._brightness
        run = 0
        for item in white:
            if item:
                run += 1
                if run >= self._width:
                    return True
            else:
                run = 0
        return False

    def borders(self, image):
        """Return list of edge names that fail the white border check."""
        pixels = np.array(image)
        strip = self._margin
        failures = []
        if np.mean(pixels[:strip, :]) <= self._brightness:
            failures.append("top")
        if np.mean(pixels[-strip:, :]) <= self._brightness:
            failures.append("bottom")
        if np.mean(pixels[:, :strip]) <= self._brightness:
            failures.append("left")
        if np.mean(pixels[:, -strip:]) <= self._brightness:
            failures.append("right")
        return failures


@final
class MangaRenderer:
    """Renders manga panel JSON into an image via Gemini image model."""

    def __init__(self, client, retries, text, border):
        self._client = client
        self._retries = retries
        self._text = text
        self._border = border

    def render(self, scene, word, progress):
        """Render scene JSON to grayscale image, retrying on validation failures."""
        dumped = json.dumps(scene, indent=2)
        panels = len(scene["manga_panel"]["panels"])
        reason = ""
        for attempt in range(self._retries):
            image = self._generate(dumped, word)
            gray = image.convert("L")
            found = self._text.detected(scene, gray)
            if found:
                reason = f"OCR detected text: '{found}'"
                progress.retry("Rendering manga", attempt + 1, reason)
                continue
            failed = self._border.borders(gray)
            if failed:
                reason = f"White border missing on: {', '.join(failed)}"
                progress.retry("Rendering manga", attempt + 1, reason)
                continue
            if panels > 1 and not self._border.gutter(gray):
                reason = "No white gutter found"
                progress.retry("Rendering manga", attempt + 1, reason)
                continue
            return gray
        raise ValueError(
            f"Rejected after {self._retries} attempts for '{word}': {reason}"
        )

    def _generate(self, prompt, word):
        """Call image model and return PIL Image."""
        response = self._client.models.generate_content(
            model="gemini-3.1-flash-image-preview",
            contents=[prompt],
            config=types.GenerateContentConfig(
                response_modalities=["IMAGE"],
                image_config=types.ImageConfig(aspect_ratio="1:1"),
                safety_settings=self._safety(),
            ),
        )
        if not response.candidates:
            raise ValueError(
                f"No candidates in image response for '{word}': "
                f"{self._diagnosis(response)}"
            )
        if not response.candidates[0].content:
            raise ValueError(f"No content in image response for '{word}'")
        for part in response.candidates[0].content.parts:
            if part.inline_data is not None:
                return Image.open(BytesIO(part.inline_data.data))
        raise ValueError(f"No image data found in response for '{word}'")

    def _safety(self):
        """Return safety settings that disable all content filters."""
        return [
            types.SafetySetting(category=category, threshold="BLOCK_NONE")
            for category in (
                "HARM_CATEGORY_HARASSMENT",
                "HARM_CATEGORY_HATE_SPEECH",
                "HARM_CATEGORY_SEXUALLY_EXPLICIT",
                "HARM_CATEGORY_DANGEROUS_CONTENT",
            )
        ]

    def _diagnosis(self, response):
        """Extract block reason and flagged safety categories from a rejected response."""
        feedback = getattr(response, "prompt_feedback", None)
        if not feedback:
            return "no prompt_feedback"
        reason = getattr(feedback.block_reason, "value", "unknown")
        message = feedback.block_reason_message or ""
        ratings = feedback.safety_ratings or []
        flagged = [
            f"{item.category.value}={item.probability.value}"
            for item in ratings
            if item.blocked or item.probability.value not in ("NEGLIGIBLE", "LOW")
        ]
        parts = [reason]
        if message:
            parts.append(message)
        if flagged:
            parts.append(f"flagged=[{', '.join(flagged)}]")
        return ", ".join(parts)
