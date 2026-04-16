//! Tests for cached audio persistence.

use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use anyhow::Result;
use hound::WavReader;
use kamishibai::infrastructure::audio::{Audio, Speaker};
use kamishibai::infrastructure::cache::Cache;
use tempfile::TempDir;

/// Counting speaker for audio generation tests.
#[derive(Clone, Debug)]
struct CountingSpeaker {
    calls: Rc<RefCell<usize>>,
    data: Vec<u8>,
}

impl CountingSpeaker {
    /// Create one counting speaker.
    fn new(data: Vec<u8>) -> Self {
        Self {
            calls: Rc::new(RefCell::new(0)),
            data,
        }
    }
}

impl Speaker for CountingSpeaker {
    /// Return one fixed PCM payload and count the call.
    fn speech(&self, _prompt: &str, _text: &str) -> Result<Vec<u8>> {
        *self.calls.borrow_mut() += 1;
        Ok(self.data.clone())
    }
}

/// Audio generation writes the expected WAV file and cache filename.
#[test]
fn audio_generation_writes_the_expected_wav_file_and_cache_filename() -> Result<()> {
    let directory = TempDir::new()?;
    let speaker = CountingSpeaker::new(vec![0, 0, 1, 0, 255, 127, 0, 128]);
    let audio = Audio::new(
        Cache::new("audio-en", directory.path()),
        "Say in natural English: {text}",
        speaker.clone(),
    );
    let (filename, cached) = audio.generate("The cat is sleeping on the windowsill")?;
    let reader = WavReader::open(audio.filepath(&filename)?)?;
    assert_eq!(
        (
            filename,
            cached,
            *speaker.calls.borrow(),
            reader.spec().channels,
            reader.spec().sample_rate,
            reader.spec().bits_per_sample
        ),
        (String::from("1cccf86c1a16.wav"), false, 1, 1, 24_000, 16),
        "audio generation no longer writes the expected WAV file and cache filename"
    );
    Ok(())
}

/// Cached audio hits do not call the speaker again.
#[test]
fn cached_audio_hits_do_not_call_the_speaker_again() -> Result<()> {
    let directory = TempDir::new()?;
    let speaker = CountingSpeaker::new(vec![0, 0, 1, 0]);
    let audio = Audio::new(
        Cache::new("audio-en", directory.path()),
        "Say in natural English: {text}",
        speaker.clone(),
    );
    let _first = audio.generate("The cat is sleeping on the windowsill")?;
    let second = audio.generate("The cat is sleeping on the windowsill")?;
    assert_eq!(
        (second.1, *speaker.calls.borrow()),
        (true, 1),
        "cached audio hits no longer skip the second speaker call"
    );
    Ok(())
}

/// Empty audio inputs keep the frozen validation message.
#[test]
fn empty_audio_inputs_keep_the_frozen_validation_message() {
    let speaker = CountingSpeaker::new(vec![0, 0, 1, 0]);
    let directory = TempDir::new().expect("temp directory must exist");
    let audio = Audio::new(
        Cache::new("audio-en", directory.path()),
        "Say in natural English: {text}",
        speaker,
    );
    assert_eq!(
        audio.generate("   ").unwrap_err().to_string(),
        "Cannot generate audio for empty text",
        "empty audio inputs no longer keep the frozen validation message"
    );
}

/// Failed audio commits remove the staged file.
#[test]
fn failed_audio_commits_remove_the_staged_file() -> Result<()> {
    let directory = TempDir::new()?;
    let cache = Cache::failing("audio-en", directory.path(), 0);
    let probe = cache.path();
    let audio = Audio::new(
        cache,
        "Say in natural English: {text}",
        CountingSpeaker::new(vec![0, 0, 1, 0]),
    );
    let _error = audio
        .generate("The cat is sleeping on the windowsill")
        .unwrap_err();
    assert!(
        fs::read_dir(probe)?.next().is_none(),
        "failed audio commits no longer remove the staged file"
    );
    Ok(())
}
