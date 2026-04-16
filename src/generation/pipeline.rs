use std::path::PathBuf;

use anyhow::Result;

use crate::vocabulary::VocabularyEntry;

use super::{
    BuildProgress, Deck, GeneratorSource, IllustrationGenerator, SkippedCard, SpeechGenerator,
};

const IMAGE_STYLE: &str = "max-width: 100%; height: auto; border-radius: 10px";

/// Orchestrate generators, deck, and progress for one batch.
pub struct DeckBuilder<M, D, P> {
    source: M,
    deck: D,
    progress: P,
}

impl<M, D, P> DeckBuilder<M, D, P> {
    /// Create one deck builder.
    pub fn new(source: M, deck: D, progress: P) -> Self {
        Self {
            source,
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

impl<M, D, P> DeckBuilder<M, D, P>
where
    M: GeneratorSource,
    D: Deck,
    P: BuildProgress,
{
    /// Process one batch and return skipped cards plus successful image payloads.
    pub fn process(
        &mut self,
        entries: &[VocabularyEntry],
    ) -> (Vec<SkippedCard>, Vec<(VocabularyEntry, PathBuf)>) {
        let mut failed = Vec::new();
        let mut processed = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            self.progress
                .card(index + 1, entries.len(), entry.term.as_str());
            let audio = match self.source.audio(entry) {
                Ok(item) => item,
                Err(error) => {
                    let reason = error.to_string();
                    self.progress.skip(entry.term.as_str(), reason.as_str());
                    failed.push(SkippedCard::new(entry.term.as_str(), reason));
                    continue;
                }
            };
            let (audiofile, audiopath) = match self.audio(entry, &audio) {
                Ok(item) => item,
                Err(error) => {
                    let reason = error.to_string();
                    self.progress.skip(entry.term.as_str(), reason.as_str());
                    failed.push(SkippedCard::new(entry.term.as_str(), reason));
                    continue;
                }
            };
            let illustration = match self.source.illustration(entry) {
                Ok(item) => item,
                Err(error) => {
                    let reason = error.to_string();
                    self.progress.skip(entry.term.as_str(), reason.as_str());
                    failed.push(SkippedCard::new(entry.term.as_str(), reason));
                    continue;
                }
            };
            let (imagefile, imagepath) = match self.image(entry, &illustration) {
                Ok(item) => item,
                Err(error) => {
                    let reason = error.to_string();
                    self.progress.skip(entry.term.as_str(), reason.as_str());
                    failed.push(SkippedCard::new(entry.term.as_str(), reason));
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
    fn audio(&mut self, entry: &VocabularyEntry, audio: &M::Audio) -> Result<(String, PathBuf)> {
        self.progress.step("Generating audio");
        let (filename, cached) = audio.generate(entry.target.sentence.as_str())?;
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
        entry: &VocabularyEntry,
        illustration: &M::Illustration,
    ) -> Result<(String, PathBuf)> {
        let (filename, _cached) = illustration.generate(
            entry.target.sentence.as_str(),
            entry.term.as_str(),
            entry.target.lang.as_str(),
            &mut self.progress,
        )?;
        Ok((filename.clone(), illustration.filepath(filename.as_str())?))
    }
}
