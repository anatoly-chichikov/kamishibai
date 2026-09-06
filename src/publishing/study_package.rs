//! Publishes completed cards as an Anki deck and printable PDF report.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tempfile::Builder;
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
        CardCell::for_draft(self.cache.clone(), draft)
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
        let models = entries
            .iter()
            .map(|entry| {
                CardModel::for_languages(entry.source.lang.as_str(), entry.target.lang.as_str())
            })
            .collect::<Result<Vec<_>>>()?;
        if !models.iter().all(|candidate| candidate == &models[0]) {
            bail!("completed cards mix incompatible text directions");
        }
        let model = models[0].model();
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
        let pdf = self.output.join(format!("{prefix}_{stamp}.pdf"));
        if apkg.exists() || pdf.exists() {
            bail!("publication target already exists for stamp '{stamp}'");
        }
        let staging = Builder::new()
            .prefix(".kamishibai-publish-")
            .tempdir_in(&self.output)?;
        let staged_apkg = staging.path().join(format!("{prefix}_{stamp}.apkg"));
        let staged_pdf = staging.path().join(format!("{prefix}_{stamp}.pdf"));
        container.save(&staged_apkg)?;
        progress.advance(PublishPhase::Report);
        report.save(&staged_pdf, &Thumbnail::new(1024))?;
        commit_publication(&staged_apkg, &apkg, &staged_pdf, &pdf)?;
        Ok(PublishedStudyPackage::new(
            apkg.to_string_lossy().into_owned(),
            pdf.to_string_lossy().into_owned(),
            self.output.to_string_lossy().into_owned(),
        ))
    }
}

fn commit_publication(
    staged_apkg: &std::path::Path,
    apkg: &std::path::Path,
    staged_pdf: &std::path::Path,
    pdf: &std::path::Path,
) -> Result<()> {
    fs::rename(staged_apkg, apkg).context("could not publish the staged Anki deck")?;
    if let Err(error) = fs::rename(staged_pdf, pdf) {
        fs::remove_file(apkg).context("could not roll back an incomplete publication")?;
        return Err(error).context("could not publish the staged printable report");
    }
    Ok(())
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

    #[test]
    fn a_failed_second_commit_rolls_back_the_first_published_file() {
        let home = TempDir::new().expect("tempdir must be created");
        let staged_apkg = home.path().join("staged.apkg");
        let staged_pdf = home.path().join("missing.pdf");
        let apkg = home.path().join("deck.apkg");
        let pdf = home.path().join("deck.pdf");
        fs::write(&staged_apkg, b"deck").expect("staged deck must be written");
        let result = commit_publication(&staged_apkg, &apkg, &staged_pdf, &pdf);
        assert_eq!(
            (result.is_err(), apkg.exists(), pdf.exists()),
            (true, false, false),
            "a failed report commit left a partial learner-facing package"
        );
    }
}
