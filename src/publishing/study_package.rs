//! Publishes completed cards as an Anki deck and printable PDF report.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use tempfile::{Builder, TempPath};
use time::OffsetDateTime;
use time::format_description::parse as parse_time;

use crate::anki::{CardModel, StableId, VocabularyDeck, VocabularyNote};
use crate::application::{PublishPhase, PublishProgress, PublishedStudyPackage, StudyPublishing};
use crate::generation::artifact_cache::{Cache, VISUAL_LOCK_TIMEOUT, VisualGuard};
use crate::generation::visual_revision;
use crate::languages::naming;
use crate::report::{CardSheet, Thumbnail};
use crate::session::{ArtifactSlot, CardCell, CardDraft, to_entry};
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

/// Writes completed cards using their producers' audio and picture paths.
/// Producers must keep those WAV and JPEG files stable throughout publication.
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
        let media = named_media(&completed, &self.cache)?;
        for (draft, [voice, image]) in completed.iter().copied().zip(media) {
            let entry = to_entry(draft)?;
            container.attach(voice.path, voice.name.as_str());
            container.attach(image.path.clone(), image.name.as_str());
            container.add(
                &entry,
                format!("[sound:{}]", voice.name).as_str(),
                format!("<img src='{}' style='{IMAGE_STYLE}'>", image.name).as_str(),
            );
            report.append(&entry, Some(image.path));
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

struct PackageMedia {
    path: PathBuf,
    name: String,
    digest: String,
}

impl PackageMedia {
    fn new(path: PathBuf, name: String, digest: String) -> Self {
        Self { path, name, digest }
    }

    fn from_artifact(draft: &CardDraft, slot: &ArtifactSlot, name: String) -> Result<Self> {
        let path = slot
            .file()
            .with_context(|| {
                format!(
                    "completed card '{}' has no {} file",
                    draft.term(),
                    slot.kind().label()
                )
            })?
            .path()
            .to_path_buf();
        let mut file = fs::File::open(&path)
            .with_context(|| format!("could not read media for card '{}'", draft.term()))?;
        let mut digest = Sha256::new();
        let mut buffer = [0; 8192];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        let digest = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(Self::new(path, name, digest))
    }
}

fn named_media(drafts: &[&CardDraft], root: &Path) -> Result<Vec<[PackageMedia; 2]>> {
    let mut media = drafts
        .iter()
        .map(|draft| {
            let cell = CardCell::for_draft(root, draft);
            Ok([
                PackageMedia::from_artifact(
                    draft,
                    draft.artifacts().sound(),
                    cell.media_name("wav"),
                )?,
                PackageMedia::from_artifact(
                    draft,
                    draft.artifacts().picture(),
                    cell.media_name("jpg"),
                )?,
            ])
        })
        .collect::<Result<Vec<_>>>()?;
    let mut contents =
        std::collections::BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    for file in media.iter().flatten() {
        contents
            .entry(file.name.clone())
            .or_default()
            .insert(file.digest.clone());
    }
    for file in media.iter_mut().flatten() {
        if contents[&file.name].len() > 1 {
            let (stem, extension) = file
                .name
                .rsplit_once('.')
                .expect("invariant: package media names always carry an extension");
            file.name = format!("{stem}-{}.{extension}", file.digest);
        }
    }
    Ok(media)
}

fn commit_publication(
    staged_apkg: &std::path::Path,
    apkg: &std::path::Path,
    staged_pdf: &std::path::Path,
    pdf: &std::path::Path,
) -> Result<()> {
    commit_file(staged_apkg, apkg).context("could not publish the staged Anki deck")?;
    if let Err(error) = commit_file(staged_pdf, pdf) {
        fs::remove_file(apkg).context("could not roll back an incomplete publication")?;
        return Err(error).context("could not publish the staged printable report");
    }
    Ok(())
}

fn commit_file(staged: &std::path::Path, destination: &std::path::Path) -> Result<()> {
    TempPath::try_from_path(staged)?.persist_noclobber(destination)?;
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
    use std::io::Read;
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;

    #[derive(Clone)]
    struct FixedClock;

    impl PublicationClock for FixedClock {
        fn stamp(&self) -> Result<String> {
            Ok(String::from("2026-09-15_131719"))
        }
    }

    struct Unwatched;

    impl PublishProgress for Unwatched {
        fn advance(&self, _phase: PublishPhase) {}
    }

    fn variant(root: &std::path::Path, name: &str, sentence: &str, shade: u8) -> CardDraft {
        use crate::session::{
            Artifact, ArtifactFile, ArtifactSlot, CardArtifacts, CardMeta, LanguagePair,
        };
        let voice = root.join(format!("{name}.wav"));
        let picture = root.join(format!("{name}.jpg"));
        fs::write(&voice, name.as_bytes()).expect("variant audio must write");
        image::RgbImage::from_pixel(8, 8, image::Rgb([shade, shade, shade]))
            .save(&picture)
            .expect("variant picture must write");
        CardDraft::new("canard", "a duck", LanguagePair::new("FR", "EN"))
            .with_meta(
                CardMeta::new(
                    "ka-nar",
                    "ka.naʁ",
                    "a duck",
                    1,
                    "The duck is here",
                    "duck",
                    "a waterbird",
                    "An animal near a pond",
                    sentence,
                ),
                None,
            )
            .with_artifacts(CardArtifacts::from_parts(
                ArtifactSlot::fresh(Artifact::Meta).succeeded(),
                ArtifactSlot::fresh(Artifact::Scene).succeeded(),
                ArtifactSlot::fresh(Artifact::Picture).succeeded_with(ArtifactFile::new(
                    "picture.jpg",
                    picture,
                    "",
                    false,
                )),
                ArtifactSlot::fresh(Artifact::Sound).succeeded_with(ArtifactFile::new(
                    "voice.wav",
                    voice,
                    "",
                    false,
                )),
            ))
    }

    fn package_media(
        path: &str,
        root: &std::path::Path,
    ) -> std::collections::BTreeMap<String, (String, Vec<u8>, String, Vec<u8>)> {
        let mut archive = zip::ZipArchive::new(fs::File::open(path).expect("package must open"))
            .expect("package must be a ZIP");
        let manifest: std::collections::BTreeMap<String, String> =
            serde_json::from_reader(archive.by_name("media").expect("manifest must exist"))
                .expect("manifest must decode");
        let media = manifest
            .into_iter()
            .map(|(index, name)| {
                let mut bytes = Vec::new();
                archive
                    .by_name(&index)
                    .expect("media must open")
                    .read_to_end(&mut bytes)
                    .expect("media must read");
                (name, bytes)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let database = root.join("collection.anki2");
        std::io::copy(
            &mut archive
                .by_name("collection.anki2")
                .expect("collection must exist"),
            &mut fs::File::create(&database).expect("collection must extract"),
        )
        .expect("collection must copy");
        let connection = rusqlite::Connection::open(database).expect("collection must open");
        let mut statement = connection
            .prepare("SELECT flds FROM notes")
            .expect("notes query must prepare");
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("notes must query")
            .map(|row| {
                let fields = row.expect("note must read");
                let fields = fields.split('\u{1f}').collect::<Vec<_>>();
                let audio = fields[6]
                    .strip_prefix("[sound:")
                    .and_then(|name| name.strip_suffix(']'))
                    .expect("audio reference must parse");
                let picture = fields[7]
                    .split('\'')
                    .nth(1)
                    .expect("picture reference must parse");
                (
                    fields[4].to_string(),
                    (
                        audio.to_string(),
                        media[audio].clone(),
                        picture.to_string(),
                        media[picture].clone(),
                    ),
                )
            })
            .collect()
    }

    #[test]
    fn variants_of_one_card_keep_their_own_audio_and_picture_in_both_batch_orders() {
        let home = TempDir::new().expect("tempdir must be created");
        let first = variant(home.path(), "first", "Le canard nage", 43);
        let second = variant(home.path(), "second", "Le canard dort", 217);
        let publisher = StudyPackagePublisher::new(
            home.path().join("cache"),
            home.path().join("forward"),
            FixedClock,
        );
        let forward = publisher
            .publish(&[first.clone(), second.clone()], &Unwatched)
            .expect("forward variants must publish")
            .into_paths()
            .0;
        let publisher = StudyPackagePublisher::new(
            home.path().join("cache"),
            home.path().join("reverse"),
            FixedClock,
        );
        let reverse = publisher
            .publish(&[second, first], &Unwatched)
            .expect("reverse variants must publish")
            .into_paths()
            .0;
        let forward = package_media(&forward, &home.path().join("forward"));
        let reverse = package_media(&reverse, &home.path().join("reverse"));
        let expected = std::collections::BTreeMap::from([
            (
                String::from("Le canard nage"),
                (
                    b"first".to_vec(),
                    fs::read(home.path().join("first.jpg")).expect("first picture must read"),
                ),
            ),
            (
                String::from("Le canard dort"),
                (
                    b"second".to_vec(),
                    fs::read(home.path().join("second.jpg")).expect("second picture must read"),
                ),
            ),
        ]);
        let actual = forward
            .iter()
            .map(|(sentence, (_, audio, _, picture))| {
                (sentence.clone(), (audio.clone(), picture.clone()))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            (actual, forward == reverse),
            (expected, true),
            "same-card variants lost their media or changed references when the batch order changed"
        );
    }

    #[test]
    fn publication_uses_the_producers_explicit_media_paths() {
        use crate::session::{
            Artifact, ArtifactFile, ArtifactSlot, CardArtifacts, CardMeta, LanguagePair,
        };
        let home = TempDir::new().expect("tempdir must be created");
        let voice = home.path().join("provider-voice.wav");
        let picture = home.path().join("provider-picture.jpg");
        fs::write(&voice, b"provider audio").expect("provider audio must write");
        image::RgbImage::from_pixel(8, 8, image::Rgb([255, 255, 255]))
            .save(&picture)
            .expect("provider picture must write");
        let draft = CardDraft::new("canard", "a duck", LanguagePair::new("FR", "EN"))
            .with_meta(
                CardMeta::new(
                    "ka-nar",
                    "ka.naʁ",
                    "a duck",
                    1,
                    "The duck is here",
                    "duck",
                    "a waterbird",
                    "An animal near a pond",
                    "Le canard est ici",
                ),
                None,
            )
            .with_artifacts(CardArtifacts::from_parts(
                ArtifactSlot::fresh(Artifact::Meta).succeeded(),
                ArtifactSlot::fresh(Artifact::Scene).succeeded(),
                ArtifactSlot::fresh(Artifact::Picture).succeeded_with(ArtifactFile::new(
                    "picture.jpg",
                    &picture,
                    "",
                    false,
                )),
                ArtifactSlot::fresh(Artifact::Sound).succeeded_with(ArtifactFile::new(
                    "voice.wav",
                    &voice,
                    "",
                    false,
                )),
            ));
        let cell = CardCell::for_draft(home.path().join("unused-cache"), &draft);
        let publisher = StudyPackagePublisher::new(
            home.path().join("unused-cache"),
            home.path().join("output"),
            FixedClock,
        );
        let published = publisher
            .publish(&[draft], &Unwatched)
            .expect("explicit artifacts must publish without conventional cache files");
        let (deck, report, _) = published.into_paths();
        let mut archive =
            zip::ZipArchive::new(fs::File::open(&deck).expect("published deck must open"))
                .expect("published deck must be a ZIP");
        let media: std::collections::BTreeMap<String, String> =
            serde_json::from_reader(archive.by_name("media").expect("media manifest must exist"))
                .expect("media manifest must decode");
        let name = media
            .iter()
            .find_map(|(index, name)| (name == &cell.media_name("wav")).then_some(index))
            .expect("audio media entry must exist");
        let mut audio = Vec::new();
        archive
            .by_name(name)
            .expect("audio entry must open")
            .read_to_end(&mut audio)
            .expect("audio bytes must read");
        assert_eq!(
            (audio, std::path::Path::new(&report).is_file()),
            (b"provider audio".to_vec(), true),
            "publication ignored explicit media paths or omitted the printable report"
        );
    }

    #[test]
    fn an_existing_deck_cannot_be_overwritten_by_a_later_commit() {
        let home = TempDir::new().expect("tempdir must be created");
        let staged_apkg = home.path().join("staged.apkg");
        let staged_pdf = home.path().join("staged.pdf");
        let apkg = home.path().join("deck.apkg");
        let pdf = home.path().join("deck.pdf");
        fs::write(&staged_apkg, b"new deck").expect("staged deck must be written");
        fs::write(&staged_pdf, b"new report").expect("staged report must be written");
        fs::write(&apkg, b"original deck").expect("original deck must be written");
        let result = commit_publication(&staged_apkg, &apkg, &staged_pdf, &pdf);
        assert_eq!(
            (
                result.is_err(),
                fs::read(&apkg).expect("deck must survive"),
                pdf.exists()
            ),
            (true, b"original deck".to_vec(), false),
            "a publication replaced a deck already owned by another run"
        );
    }

    #[test]
    fn an_existing_report_survives_rollback_of_a_competing_publication() {
        let home = TempDir::new().expect("tempdir must be created");
        let staged_apkg = home.path().join("staged.apkg");
        let staged_pdf = home.path().join("staged.pdf");
        let apkg = home.path().join("deck.apkg");
        let pdf = home.path().join("deck.pdf");
        fs::write(&staged_apkg, b"new deck").expect("staged deck must be written");
        fs::write(&staged_pdf, b"new report").expect("staged report must be written");
        fs::write(&pdf, b"original report").expect("original report must be written");
        let result = commit_publication(&staged_apkg, &apkg, &staged_pdf, &pdf);
        assert_eq!(
            (
                result.is_err(),
                apkg.exists(),
                fs::read(&pdf).expect("report must survive")
            ),
            (true, false, b"original report".to_vec()),
            "a failed competing publication replaced an existing report or left its own deck"
        );
    }

    #[test]
    fn simultaneous_publications_cannot_mix_or_replace_the_winning_package() {
        let home = TempDir::new().expect("tempdir must be created");
        let apkg = home.path().join("deck.apkg");
        let pdf = home.path().join("deck.pdf");
        let barrier = Arc::new(Barrier::new(3));
        let (sender, receiver) = mpsc::channel();
        let threads = [b"first".as_slice(), b"second".as_slice()]
            .into_iter()
            .enumerate()
            .map(|(index, bytes)| {
                let staged_apkg = home.path().join(format!("staged-{index}.apkg"));
                let staged_pdf = home.path().join(format!("staged-{index}.pdf"));
                fs::write(&staged_apkg, bytes).expect("staged deck must be written");
                fs::write(&staged_pdf, bytes).expect("staged report must be written");
                let apkg = apkg.clone();
                let pdf = pdf.clone();
                let barrier = barrier.clone();
                let sender = sender.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let result = commit_publication(&staged_apkg, &apkg, &staged_pdf, &pdf);
                    sender
                        .send(result.is_ok())
                        .expect("commit result must be delivered");
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let first = receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("first commit must finish");
        let second = receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("second commit must finish");
        for thread in threads {
            thread.join().expect("publication thread must exit");
        }
        assert_eq!(
            (
                usize::from(first) + usize::from(second),
                fs::read(&apkg).expect("deck must exist")
                    == fs::read(&pdf).expect("report must exist")
            ),
            (1, true),
            "concurrent publication replaced the winner or paired files from different runs"
        );
    }

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
