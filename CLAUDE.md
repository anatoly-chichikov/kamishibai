# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**vocabulary-anki** is a Python 3.9+ application that converts vocabulary data from JSON format into Anki flashcard decks with AI-generated audio pronunciations and manga-style illustrations.

## Development Setup and Commands

### Installation and Running

```bash
# Install dependencies (uses uv for dependency management)
uv sync

# Run the application
uv run create_anki_deck.py
```

### Required Environment

- **GEMINI_API_KEY**: Set this environment variable with your Google Generative AI API key. The application will fail at startup if this is not configured.

## Architecture Overview

The application follows a linear pipeline architecture with four main components:

### Component Flow

```
JSON File → JsonReader → AudioGenerator & ImageGenerator → VocabularyDeck → Anki Package (.apkg)
```

### Key Classes

**JsonReader** (`create_anki_deck.py:271-299`)
- Reads vocabulary entries from JSON format
- Expected fields: `word`, `pronunciation`, `translation_ru`, `sentence_en`, `sentence_ru`, `context_ru`, `importance`
- Returns list of entry dictionaries
- Filters out incomplete entries (requires both word and Russian sentence)

**AudioGenerator** (`create_anki_deck.py:19-67`)
- Generates audio files from English sentences using Google Gemini 2.5 Flash TTS
- Uses "Kore" voice with natural English pronunciation
- Outputs 24kHz mono WAV files
- Implements 10-attempt retry logic with 60-second delays for rate limiting (HTTP 429, RESOURCE_EXHAUSTED errors)

**ImageGenerator** (`create_anki_deck.py:70-137`)
- Generates black and white manga-style illustrations from English sentences
- Uses Google Gemini 2.5 Flash Image model
- 1:1 aspect ratio PNG output
- Safety settings set to BLOCK_NONE for educational content (intentional)
- Returns HTML img tag for Anki integration or empty string if generation fails
- Same 10-attempt retry logic with 60-second delays

**VocabularyDeck** (`create_anki_deck.py:140-223`)
- Assembles all data into Anki deck format using genanki library
- Creates a note model with 9 fields: RussianSentence, Word, Pronunciation, Translation, Example, Importance, Audio, Image, Context
- Card template displays Russian sentence and image on front; audio, English example, pronunciation, translation, importance rating, and context on back
- Manages media file attachment and final .apkg export

### Entry Point

The `main()` function (`create_anki_deck.py:257-296`) orchestrates the entire pipeline:
1. Loads GEMINI_API_KEY from environment
2. Creates temporary directory for media files
3. Reads entries from hardcoded path: `/Users/chichikov/Downloads/vocabulary.csv`
4. For each entry: generates audio, generates image, creates flashcard
5. Exports to: `/Users/chichikov/Downloads/vocabulary.apkg`

## Important Implementation Details

### File Path Configuration

Currently hardcoded in `main()`:
- **Input**: `/Users/chichikov/Downloads/vocabulary.json` (line 319)
- **Output**: `/Users/chichikov/Downloads/vocabulary.apkg` (line 358)

These paths are not parameterized.

### Rate Limiting

Both AudioGenerator and ImageGenerator implement retry logic for Google Gemini API rate limits:
- Detects 429 HTTP errors and RESOURCE_EXHAUSTED errors
- Retries up to 10 times with 60-second delays between attempts
- Each card can take up to 10 minutes if rate limiting occurs

### Media File Management

- Uses `tempfile.mkdtemp()` for temporary directory
- Media files are referenced in Anki cards using `[sound:filename.wav]` and `<img src='filename.png'>` syntax
- All media files are attached to the deck before exporting to .apkg

### Card Data Model

Each flashcard contains:
- **Front**: Russian sentence + manga illustration
- **Back**: Audio pronunciation + English example sentence + word + pronunciation guide + Russian translation + importance rating (1-10) + Russian context

## Dependencies

- **genanki** (≥0.13.0): Anki deck format generation
- **google-genai** (≥0.1.0): Google Generative AI API client
- **pillow** (≥10.0.0): Image processing and PNG saving
