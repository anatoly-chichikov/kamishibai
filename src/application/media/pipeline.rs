use std::path::PathBuf;

use anyhow::Result;

use crate::domain::entry::NormalizedEntry;

use super::{AudioService, Deck, Failure, IllustrationService, MediaSource, PipelineProgress};

const IMAGE_STYLE: &str = "max-width: 100%; height: auto; border-radius: 10px";

/// Orchestrate media, deck, and progress for one batch.
pub struct Pipeline<M, D, P> {
    media: M,
    deck: D,
    progress: P,
}

impl<M, D, P> Pipeline<M, D, P> {
    /// Create one media pipeline.
    pub fn new(media: M, deck: D, progress: P) -> Self {
        Self {
            media,
            deck,
            progress,
        }
    }

    /// Return the accumulated deck.
    pub fn deck(&self) -> &D {
        &self.deck
    }

    /// Return the accumulated progress recorder.
    pub fn progress(&self) -> &P {
        &self.progress
    }

    /// Return the accumulated progress recorder mutably.
    pub fn progress_mut(&mut self) -> &mut P {
        &mut self.progress
    }
}

impl<M, D, P> Pipeline<M, D, P>
where
    M: MediaSource,
    D: Deck,
    P: PipelineProgress,
{
    /// Process one batch and return failures plus successful image payloads.
    pub fn process(
        &mut self,
        entries: &[NormalizedEntry],
    ) -> (Vec<Failure>, Vec<(NormalizedEntry, PathBuf)>) {
        let mut failed = Vec::new();
        let mut processed = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            self.progress
                .card(index + 1, entries.len(), entry.word.as_str());
            let audio = match self.media.audio(entry) {
                Ok(item) => item,
                Err(error) => {
                    let reason = error.to_string();
                    self.progress.skip(entry.word.as_str(), reason.as_str());
                    failed.push(Failure::new(entry.word.clone(), reason));
                    continue;
                }
            };
            let (audiofile, audiopath) = match self.audio(entry, &audio) {
                Ok(item) => item,
                Err(error) => {
                    let reason = error.to_string();
                    self.progress.skip(entry.word.as_str(), reason.as_str());
                    failed.push(Failure::new(entry.word.clone(), reason));
                    continue;
                }
            };
            let illustration = match self.media.illustration(entry) {
                Ok(item) => item,
                Err(error) => {
                    let reason = error.to_string();
                    self.progress.skip(entry.word.as_str(), reason.as_str());
                    failed.push(Failure::new(entry.word.clone(), reason));
                    continue;
                }
            };
            let (imagefile, imagepath) = match self.image(entry, &illustration) {
                Ok(item) => item,
                Err(error) => {
                    let reason = error.to_string();
                    self.progress.skip(entry.word.as_str(), reason.as_str());
                    failed.push(Failure::new(entry.word.clone(), reason));
                    continue;
                }
            };
            self.deck.attach(audiopath.as_path());
            self.deck.attach(imagepath.as_path());
            self.deck.add(
                entry,
                format!("[sound:{audiofile}]").as_str(),
                format!("<img src='{imagefile}' style='{IMAGE_STYLE}'>").as_str(),
            );
            processed.push((entry.clone(), imagepath));
        }
        (failed, processed)
    }

    /// Generate audio and return the filename plus absolute path.
    fn audio(&mut self, entry: &NormalizedEntry, audio: &M::Audio) -> Result<(String, PathBuf)> {
        self.progress.step("Generating audio");
        let (filename, cached) = audio.generate(entry.example.as_str())?;
        let path = audio.filepath(filename.as_str())?;
        self.progress.done(
            "Generating audio",
            if cached { "cached" } else { "generated" },
            Some(path.as_path()),
        );
        Ok((filename, path))
    }

    /// Generate one illustration and return the filename plus absolute path.
    fn image(
        &mut self,
        entry: &NormalizedEntry,
        illustration: &M::Illustration,
    ) -> Result<(String, PathBuf)> {
        let (filename, _cached) = illustration.generate(
            entry.example.as_str(),
            entry.word.as_str(),
            entry.target_lang.as_str(),
            &mut self.progress,
        )?;
        Ok((filename.clone(), illustration.filepath(filename.as_str())?))
    }
}
