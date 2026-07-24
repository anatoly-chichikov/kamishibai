//! Publishes completed cards as an Anki deck and printable PDF report.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};
use time::OffsetDateTime;
use time::format_description::parse as parse_time;

use crate::anki::{CardModel, StableId, VocabularyDeck, VocabularyNote};
use crate::application::{PublishPhase, PublishProgress, PublishedStudyPackage, StudyPublishing};
use crate::generation::artifact_cache::{
    Cache, ILLUSTRATION_FILE, VISUAL_LOCK_TIMEOUT, VOICE_FILE, VisualGuard,
};
use crate::generation::visual_revision;
use crate::languages::naming;
use crate::report::{CardSheet, Thumbnail};
use crate::session::{CardCell, CardDraft, to_entry};
use crate::vocabulary::VocabularyEntry;

const IMAGE_STYLE: &str = "max-width: 100%; height: auto; border-radius: 10px";

/// Supplies the timestamp embedded in learner-facing package filenames.
pub(crate) trait PublicationClock: Clone + Send + 'static {
    /// Return one filename-safe UTC publication stamp.
    fn stamp(&self) -> Result<String>;
}

/// Reads publication timestamps from the system UTC clock.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SystemPublicationClock;

impl PublicationClock for SystemPublicationClock {
    fn stamp(&self) -> Result<String> {
        Ok(OffsetDateTime::now_utc()
            .format(parse_time("[year]-[month]-[day]_[hour][minute][second]")?.as_slice())?)
    }
}

/// Writes the completed subset of cards to the configured output directory.
#[derive(Clone, Debug)]
pub(crate) struct StudyPackagePublisher<C = SystemPublicationClock> {
    cache: PathBuf,
    output: PathBuf,
    clock: C,
}

impl<C> StudyPackagePublisher<C> {
    /// Bind publication to the shared cache, output directory, and clock.
    #[must_use]
    pub(crate) fn new(cache: PathBuf, output: PathBuf, clock: C) -> Self {
        Self {
            cache,
            output,
            clock,
        }
    }

    fn cell(&self, draft: &CardDraft) -> CardCell {
        CardCell::new(
            self.cache.clone(),
            draft.pair(),
            draft.term(),
            draft.understanding(),
        )
    }
}

impl<C> StudyPublishing for StudyPackagePublisher<C>
where
    C: PublicationClock,
{
    fn publish(
        &self,
        drafts: &[CardDraft],
        progress: &dyn PublishProgress,
    ) -> Result<PublishedStudyPackage> {
        progress.advance(PublishPhase::Deck);
        fs::create_dir_all(&self.output)?;
        let completed = drafts
            .iter()
            .filter(|draft| draft.artifacts().all_ready())
            .collect::<Vec<_>>();
        let entries: Vec<VocabularyEntry> = completed
            .iter()
            .copied()
            .map(to_entry)
            .collect::<Result<Vec<_>>>()?;
        if entries.is_empty() {
            bail!("no completed cards to publish");
        }
        let decknaming = naming(None, entries.as_slice());
        let model = CardModel::new().model();
        let mut container = VocabularyDeck::new(
            StableId::new(decknaming.name.as_str()).value(),
            decknaming.name.as_str(),
            VocabularyNote::new(model),
            Vec::<(PathBuf, String)>::new(),
        );
        let mut report = CardSheet::new();
        let visuals = completed
            .iter()
            .copied()
            .map(|draft| self.cell(draft).cache().visual(visual_revision()))
            .collect::<Result<Vec<_>>>()?;
        let _guards = hold_visuals(visuals, VISUAL_LOCK_TIMEOUT)?;
        for draft in completed.iter().copied() {
            let entry = to_entry(draft)?;
            let cell = self.cell(draft);
            let cache = cell.cache();
            let visual = cache.visual(visual_revision())?;
            let voice = cell.media_name("wav");
            let image = cell.media_name("jpg");
            let voice_path = cache.filepath(VOICE_FILE)?;
            let image_path = visual.filepath(ILLUSTRATION_FILE)?;
            container.attach(voice_path, voice.as_str());
            container.attach(image_path.clone(), image.as_str());
            container.add(
                &entry,
                format!("[sound:{voice}]").as_str(),
                format!("<img src='{image}' style='{IMAGE_STYLE}'>").as_str(),
            );
            report.append(&entry, Some(image_path));
        }
        let stamp = self.clock.stamp()?;
        let prefix = decknaming.prefix.to_uppercase();
        let apkg = self.output.join(format!("{prefix}_{stamp}.apkg"));
        container.save(&apkg)?;
        progress.advance(PublishPhase::Report);
        let pdf = self.output.join(format!("{prefix}_{stamp}.pdf"));
        report.save(&pdf, &Thumbnail::new(1024))?;
        Ok(PublishedStudyPackage::new(
            apkg.to_string_lossy().into_owned(),
            pdf.to_string_lossy().into_owned(),
            self.output.to_string_lossy().into_owned(),
        ))
    }
}

fn hold_visuals(mut visuals: Vec<Cache>, timeout: Duration) -> Result<Vec<VisualGuard>> {
    visuals.sort_by_key(Cache::path);
    visuals.dedup_by(|left, right| left.path() == right.path());
    visuals
        .iter()
        .map(|visual| visual.hold_visual(timeout))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn duplicate_visual_paths_hold_one_lock_without_deadlocking() {
        let home = TempDir::new().expect("tempdir must be created");
        let cache = Cache::new("cards/test", home.path())
            .visual("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .expect("visual cache must resolve");
        let guards = hold_visuals(vec![cache.clone(), cache], Duration::ZERO)
            .expect("duplicate visual paths must acquire one lock");
        assert_eq!(
            guards.len(),
            1,
            "duplicate visual paths acquired the same non-reentrant lock twice"
        );
    }
}
