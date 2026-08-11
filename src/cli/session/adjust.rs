//! Headless staging of sentence-label and rewrite-note changes.

use std::path::Path;

use anyhow::Result;

use crate::cli::error::{not_ready_hint, usage, usage_hint};
use crate::runtime::locations::{SystemContext, cache_root};
use crate::session::{
    CardDraft, CardMeta, CardMetaCache, CardRewrite, LanguagePair, SentenceAxis,
    SentenceLabelSelection,
};

use super::args::{AdjustArgs, AdjustKind, AdjustLevel, AdjustRegister, RestoreAxis};
use super::store::{DraftRecord, Phase, SessionRecord, SessionStore};
use super::{Render, json, refuse_if_live, resolve, view};

/// Stage one card's sentence-label or note changes without starting generation.
pub(super) fn adjust(args: &AdjustArgs, render: Render) -> Result<()> {
    let store = SessionStore::system()?;
    let record = resolve(&store, args.id.as_deref(), render)?;
    let root = cache_root(&SystemContext)?;
    let updated = store.update(record.id.as_str(), |fresh| {
        refuse_if_live(&store, fresh)?;
        if matches!(fresh.phase, Phase::Generating) {
            return Err(usage(format!(
                "session '{}' is starting generation; wait or cancel it first",
                fresh.id
            )));
        }
        let slot = target(fresh, args)?;
        let current = fresh.drafts[slot].clone();
        if current.rewrite.as_ref().is_some_and(CardRewrite::started) {
            return Err(usage_hint(
                format!("card '{}' already has an active rewrite", current.term),
                format!("Wait for it or resume it: kamishibai generate {}", fresh.id),
            ));
        }
        let pair = LanguagePair::new(fresh.learning.as_str(), fresh.known.as_str());
        let Some(meta) = current_meta(root.as_path(), &current, &pair)? else {
            return Err(not_ready_hint(
                format!(
                    "card '{}' has no generated metadata to adjust",
                    current.term
                ),
                format!("Generate it first: kamishibai generate {}", fresh.id),
            ));
        };
        fresh.drafts[slot].rewrite = staged(&current, meta, pair, args);
        Ok(())
    })?;
    if matches!(render, Render::Json) {
        return json::emit_session(&updated);
    }
    let pending = updated
        .drafts
        .iter()
        .filter(|draft| {
            draft
                .rewrite
                .as_ref()
                .is_some_and(|rewrite| !rewrite.started())
        })
        .count();
    println!("{}", view::header(&updated, updated.phase));
    println!("adjusted {} — {pending} pending", args.card);
    if pending > 0 {
        println!("next: kamishibai regenerate {} --pending", updated.id);
    }
    Ok(())
}

fn target(record: &SessionRecord, args: &AdjustArgs) -> Result<usize> {
    let matches = record
        .drafts
        .iter()
        .enumerate()
        .filter(|(_, draft)| {
            draft.term == args.card
                && args
                    .understanding
                    .as_ref()
                    .is_none_or(|understanding| draft.understanding == *understanding)
        })
        .map(|(slot, _)| slot)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [slot] => Ok(*slot),
        [] => Err(usage(format!(
            "no matching card '{}' in session '{}'",
            args.card, record.id
        ))),
        _ if args.understanding.is_none() => Err(usage_hint(
            format!("card '{}' has more than one selected sense", args.card),
            "Pass --understanding TEXT to choose one",
        )),
        _ => Err(usage(format!(
            "card '{}' with that understanding is still ambiguous in session '{}'",
            args.card, record.id
        ))),
    }
}

fn current_meta(root: &Path, draft: &DraftRecord, pair: &LanguagePair) -> Result<Option<CardMeta>> {
    if let Some(meta) = draft
        .rewrite
        .as_ref()
        .and_then(CardRewrite::previous)
        .cloned()
    {
        return Ok(Some(meta));
    }
    CardMetaCache::new(root.to_path_buf()).load(
        draft.term.as_str(),
        draft.understanding.as_str(),
        pair,
    )
}

fn staged(
    current: &DraftRecord,
    meta: CardMeta,
    pair: LanguagePair,
    args: &AdjustArgs,
) -> Option<CardRewrite> {
    let baseline = meta
        .sentence_labels()
        .map(SentenceLabelSelection::from_labels)
        .unwrap_or_default();
    let mut selection = current
        .rewrite
        .as_ref()
        .map(|rewrite| rewrite.selection().clone())
        .unwrap_or_else(|| baseline.clone());
    let note = args.note.clone().unwrap_or_else(|| {
        current
            .rewrite
            .as_ref()
            .map(|rewrite| rewrite.note().to_string())
            .unwrap_or_default()
    });
    for restore in &args.restore {
        for axis in restore.axes() {
            selection = selection.restoring(axis, &baseline);
        }
    }
    if let Some(register) = args.register {
        selection = chosen(
            selection,
            &baseline,
            SentenceAxis::Register,
            register.index(),
        );
    }
    if let Some(kind) = args.kind {
        selection = chosen(selection, &baseline, SentenceAxis::Type, kind.index());
    }
    if let Some(level) = args.level {
        selection = chosen(selection, &baseline, SentenceAxis::Level, level.index());
    }
    CardDraft::new(current.term.as_str(), current.understanding.as_str(), pair)
        .with_meta(meta, None)
        .with_rewrite(current.rewrite.clone())
        .staging_rewrite(selection, note)
        .rewrite()
        .cloned()
}

fn chosen(
    selection: SentenceLabelSelection,
    baseline: &SentenceLabelSelection,
    axis: SentenceAxis,
    index: usize,
) -> SentenceLabelSelection {
    let selected = selection.choosing(axis, index);
    if selected.token(axis) == baseline.token(axis) {
        return selected.restoring(axis, baseline);
    }
    selected
}

impl AdjustRegister {
    fn index(self) -> usize {
        match self {
            Self::Neutral => 0,
            Self::Casual => 1,
            Self::Formal => 2,
            Self::Literary => 3,
            Self::Archaic => 4,
        }
    }
}

impl AdjustKind {
    fn index(self) -> usize {
        match self {
            Self::Statement => 0,
            Self::Question => 1,
            Self::Request => 2,
            Self::Exclamation => 3,
            Self::Dialogue => 4,
        }
    }
}

impl AdjustLevel {
    fn index(self) -> usize {
        match self {
            Self::A1 => 0,
            Self::A2 => 1,
            Self::B1 => 2,
            Self::B2 => 3,
            Self::C1 => 4,
            Self::C2 => 5,
        }
    }
}

impl RestoreAxis {
    fn axes(self) -> Vec<SentenceAxis> {
        match self {
            Self::Register => vec![SentenceAxis::Register],
            Self::Level => vec![SentenceAxis::Level],
            Self::Kind => vec![SentenceAxis::Type],
            Self::All => vec![
                SentenceAxis::Register,
                SentenceAxis::Level,
                SentenceAxis::Type,
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{
        ArtifactCosts, AxisSet, Register, SentenceKind, SentenceLabels, SentenceLevel,
    };

    fn meta() -> CardMeta {
        CardMeta::new(
            "/bank/",
            "/a bank/",
            "a financial institution",
            7,
            "The bank approved it.",
            "bank",
            "money institution",
            "ordinary finance",
            "La banque l'a approuve.",
        )
        .with_sentence_labels(SentenceLabels::new(
            Register::Neutral,
            SentenceLevel::A2,
            SentenceKind::Statement,
            AxisSet::default(),
            AxisSet::default(),
        ))
    }

    fn record(rewrite: Option<CardRewrite>) -> DraftRecord {
        DraftRecord {
            term: String::from("bank"),
            understanding: String::from("a financial institution"),
            costs: ArtifactCosts::default(),
            rewrite,
            meta_request: None,
        }
    }

    fn args() -> AdjustArgs {
        AdjustArgs {
            id: Some(String::from("fr-1")),
            card: String::from("bank"),
            understanding: None,
            register: None,
            kind: None,
            level: None,
            restore: Vec::new(),
            note: None,
        }
    }

    #[test]
    fn repeated_adjustments_accumulate_and_preserve_an_omitted_note() {
        let mut first = args();
        first.register = Some(AdjustRegister::Formal);
        first.note = Some(String::from("keep it short"));
        let first = staged(&record(None), meta(), LanguagePair::new("FR", "EN"), &first);
        let mut second = args();
        second.level = Some(AdjustLevel::B1);
        let second = staged(
            &record(first),
            meta(),
            LanguagePair::new("FR", "EN"),
            &second,
        )
        .expect("the accumulated rewrite must remain staged");
        assert_eq!(
            (
                second.selection().register(),
                second.selection().level(),
                second.selection().kind(),
                second.note(),
                second.started(),
            ),
            (
                Some(Register::Formal),
                Some(SentenceLevel::B1),
                Some(SentenceKind::Statement),
                "keep it short",
                false,
            ),
            "a later adjust command overwrote an earlier axis or omitted note"
        );
    }

    #[test]
    fn restoring_every_axis_and_clearing_the_note_removes_pending_state() {
        let selection = SentenceLabelSelection::from_labels(
            meta()
                .sentence_labels()
                .expect("labeled metadata must expose its baseline"),
        )
        .choosing(SentenceAxis::Register, 2)
        .choosing(SentenceAxis::Level, 2);
        let pending = CardDraft::new(
            "bank",
            "a financial institution",
            LanguagePair::new("FR", "EN"),
        )
        .with_meta(meta(), None)
        .staging_rewrite(selection, "keep it short")
        .rewrite()
        .cloned();
        let mut restore = args();
        restore.restore = vec![RestoreAxis::All];
        restore.note = Some(String::new());
        let restored = staged(
            &record(pending),
            meta(),
            LanguagePair::new("FR", "EN"),
            &restore,
        );
        assert_eq!(
            restored, None,
            "restoring the generated preset with an empty note left a phantom pending rewrite"
        );
    }

    #[test]
    fn understanding_disambiguates_repeated_terms() {
        let mut session = SessionRecord::understood(
            String::from("fr-1"),
            String::from("created"),
            String::from("EN"),
            String::from("FR"),
            String::from("/out"),
            String::from("all"),
            String::from("words"),
            Vec::new(),
            Vec::new(),
        );
        session.drafts = vec![
            record(None),
            DraftRecord {
                term: String::from("bank"),
                understanding: String::from("the side of a river"),
                costs: ArtifactCosts::default(),
                rewrite: None,
                meta_request: None,
            },
        ];
        let mut selected = args();
        selected.understanding = Some(String::from("the side of a river"));
        assert_eq!(
            target(&session, &selected).expect("understanding must select one sense"),
            1,
            "adjust targeted the wrong sense of a repeated term"
        );
    }
}
