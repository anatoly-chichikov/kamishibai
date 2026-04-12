"""Media generation and pipeline orchestration for kamishibai."""

import hashlib
import json
import os
import random
import wave
from typing import Protocol, final

from google.genai import types


class Voice(Protocol):
    """Protocol for TTS voice configuration."""

    def speech(self):
        """Return a SpeechConfig for TTS generation."""
        ...

    def models(self):
        """Return tuple of model names for fallback iteration."""
        ...


@final
class TtsVoice:
    """Represents a TTS voice configuration with fallback models."""

    _VOICES = (
        "Achernar", "Achird", "Algenib", "Algieba", "Alnilam",
        "Aoede", "Autonoe", "Callirrhoe", "Charon", "Despina",
        "Enceladus", "Erinome", "Fenrir", "Gacrux", "Iapetus",
        "Kore", "Laomedeia", "Leda", "Orus", "Puck",
        "Pulcherrima", "Rasalgethi", "Sadachbia", "Sadaltager",
        "Schedar", "Sulafat", "Umbriel", "Vindemiatrix", "Zephyr",
        "Zubenelgenubi",
    )

    def __init__(self, models):
        self._models = models

    def speech(self):
        """Return SpeechConfig with a randomly chosen voice."""
        return types.SpeechConfig(
            voice_config=types.VoiceConfig(
                prebuilt_voice_config=types.PrebuiltVoiceConfig(
                    voice_name=random.choice(self._VOICES),
                )
            )
        )

    def models(self):
        """Return tuple of model names for fallback iteration."""
        return self._models

    @staticmethod
    def pool():
        """Return the tuple of all available voice names."""
        return TtsVoice._VOICES


@final
class Audio:
    """Generates audio files from text using Gemini TTS."""

    def __init__(self, client, cache, prompt, voice):
        self._client = client
        self._cache = cache
        self._prompt = prompt
        self._voice = voice

    def filepath(self, filename):
        """Return full path to a cached audio file."""
        return self._cache.filepath(filename)

    def generate(self, text):
        """Generate audio file and return tuple of filename and cached flag."""
        if not text.strip():
            raise ValueError("Cannot generate audio for empty text")
        digest = hashlib.md5(text.encode()).hexdigest()[:12]
        filename = f"{digest}.wav"
        if self._cache.exists(filename):
            return (filename, True)
        data = self._speech(self._prompt.format(text=text), text)
        self._commit(data, filename)
        return (filename, False)

    def _speech(self, prompt, text):
        """Call TTS model with fallback and return raw audio bytes."""
        for model in self._voice.models():
            try:
                response = self._client.models.generate_content(
                    model=model,
                    contents=prompt,
                    config=types.GenerateContentConfig(
                        response_modalities=["AUDIO"],
                        speech_config=self._voice.speech(),
                    ),
                )
                if not response.candidates:
                    raise ValueError(f"No candidates in audio response for '{text}'")
                if not response.candidates[0].content:
                    raise ValueError(f"No content in audio response for '{text}'")
                return response.candidates[0].content.parts[0].inline_data.data
            except Exception as error:
                if "RESOURCE_EXHAUSTED" in str(error):
                    continue
                raise
        raise ValueError(f"Failed to generate audio on all models for '{text}'")

    def _commit(self, data, filename):
        """Write WAV data to cache atomically."""
        staged = self._cache.stage(".wav")
        try:
            with wave.open(staged, "wb") as item:
                item.setnchannels(1)
                item.setsampwidth(2)
                item.setframerate(24000)
                item.writeframes(data)
            self._cache.commit(staged, filename)
        except BaseException:
            if os.path.exists(staged):
                os.remove(staged)
            raise


@final
class Illustration:
    """Generates manga images via a two-step pipeline."""

    def __init__(self, cache, translator, renderer):
        self._cache = cache
        self._translator = translator
        self._renderer = renderer

    def filepath(self, filename):
        """Return full path to a cached illustration file."""
        return self._cache.filepath(filename)

    def generate(self, sentence, word, target, progress):
        """Generate manga image and return tuple of filename and cached flag."""
        digest = hashlib.md5(f"{target}\0{sentence}".encode()).hexdigest()[:12]
        filename = f"{digest}.jpg"
        scenefile = f"{digest}.json"
        path = self._cache.filepath(filename)
        if self._cache.exists(filename):
            self._cached(scenefile, progress)
            progress.done("Rendering manga", "cached", path)
            return (filename, True)
        scene = self._scene(sentence, target, scenefile, progress)
        progress.step("Rendering manga")
        image = self._renderer.render(scene, word, progress)
        self._commit(image, filename)
        progress.done("Rendering manga", "rendered", path)
        return (filename, False)

    def _cached(self, scenefile, progress):
        """Report scene cache status when image is already cached."""
        if self._cache.exists(scenefile):
            progress.done("Composing scene", "cached", self._cache.filepath(scenefile))
        else:
            progress.done("Composing scene", "cached")

    def _scene(self, sentence, target, scenefile, progress):
        """Load scene from cache or translate and cache it."""
        scenepath = self._cache.filepath(scenefile)
        progress.step("Composing scene")
        if self._cache.exists(scenefile):
            with open(scenepath, "r", encoding="utf-8") as item:
                scene = json.load(item)
            progress.done("Composing scene", "cached", scenepath)
            return scene
        scene = self._translator.translate(sentence, target)
        staged = self._cache.stage(".json")
        try:
            with open(staged, "w", encoding="utf-8") as item:
                json.dump(scene, item, indent=2, ensure_ascii=False)
            self._cache.commit(staged, scenefile)
        except BaseException:
            if os.path.exists(staged):
                os.remove(staged)
            raise
        progress.done("Composing scene", "translated", scenepath)
        return scene

    def _commit(self, image, filename):
        """Write image to cache atomically."""
        staged = self._cache.stage(".jpg")
        try:
            image.save(staged, "JPEG", quality=60)
            self._cache.commit(staged, filename)
        except BaseException:
            if os.path.exists(staged):
                os.remove(staged)
            raise


@final
class Pipeline:
    """Orchestrates audio and image generation for each entry."""

    def __init__(self, audio, illustration, deck, progress):
        self._audio = audio
        self._illustration = illustration
        self._deck = deck
        self._progress = progress

    def _audio_service(self, entry):
        """Return the audio generator for the given entry."""
        if hasattr(self._audio, "audio"):
            return self._audio.audio(entry)
        return self._audio

    def _illustration_service(self, entry):
        """Return the illustration generator for the given entry."""
        if hasattr(self._illustration, "illustration"):
            return self._illustration.illustration(entry)
        return self._illustration

    def process(self, entries):
        """Process all entries and return tuple of failures and processed list."""
        failed = []
        processed = []
        for index, entry in enumerate(entries, 1):
            self._progress.card(index, len(entries), entry["word"])
            audio = self._audio_service(entry)
            try:
                self._progress.step("Generating audio")
                audiofile, cached = audio.generate(entry["example"])
                audiopath = audio.filepath(audiofile)
                label = "cached" if cached else "generated"
                self._progress.done("Generating audio", label, audiopath)
            except Exception as error:
                self._progress.skip(entry["word"], str(error))
                failed.append({"word": entry["word"], "reason": str(error)})
                continue
            illustration = self._illustration_service(entry)
            try:
                imagefile, cached = illustration.generate(
                    entry["example"], entry["word"], entry["target_lang"], self._progress
                )
            except Exception as error:
                self._progress.skip(entry["word"], str(error))
                failed.append({"word": entry["word"], "reason": str(error)})
                continue
            self._deck.attach(audiopath)
            imagepath = illustration.filepath(imagefile)
            self._deck.attach(imagepath)
            processed.append((entry, imagepath))
            sound = f"[sound:{audiofile}]"
            style = "max-width: 100%; height: auto; border-radius: 10px"
            html = f"<img src='{imagefile}' style='{style}'>"
            self._deck.add(entry, sound, html)
        return (failed, processed)
