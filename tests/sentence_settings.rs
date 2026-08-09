use kamishibai::session::{
    ArtifactCosts, CardDraft, CardMeta, LanguagePair, SentenceAxis, SentenceBatchSettings,
    SentenceKind, SentenceLevel, SentenceTypeMix,
};

#[test]
fn natural_defaults_add_no_initial_label_request() {
    assert_eq!(
        SentenceBatchSettings::default().selections(7),
        vec![None; 7],
        "default sentence settings introduced a pinned generation request"
    );
}

#[test]
fn a_batch_level_is_pinned_on_every_card() {
    let selections =
        SentenceBatchSettings::new(Some(SentenceLevel::B1), SentenceTypeMix::Natural).selections(7);
    assert!(
        selections.iter().all(|selection| {
            selection.as_ref().is_some_and(|selection| {
                selection.level() == Some(SentenceLevel::B1)
                    && selection.pinned().contains(SentenceAxis::Level)
                    && selection.kind().is_none()
            })
        }),
        "batch level was not the only request pinned on every card"
    );
}

#[test]
fn varied_types_are_deterministic_weighted_and_never_forced() {
    let settings = SentenceBatchSettings::new(None, SentenceTypeMix::Varied);
    let first = settings.selections(100);
    let second = settings.selections(100);
    let kinds = first
        .iter()
        .filter_map(|selection| selection.as_ref().and_then(|selection| selection.kind()))
        .collect::<Vec<_>>();
    let statements = kinds
        .iter()
        .filter(|kind| **kind == SentenceKind::Statement)
        .count();
    let questions = kinds
        .iter()
        .filter(|kind| **kind == SentenceKind::Question)
        .count();
    let dialogues = kinds
        .iter()
        .filter(|kind| **kind == SentenceKind::Dialogue)
        .count();
    assert_eq!(
        (
            first == second,
            statements,
            questions,
            dialogues,
            kinds.iter().all(|kind| matches!(
                kind,
                SentenceKind::Statement | SentenceKind::Question | SentenceKind::Dialogue
            )),
        ),
        (true, 60, 20, 20, true),
        "varied allocation was unstable, unweighted, or used a forced sentence type"
    );
}

#[test]
fn an_initial_meta_request_survives_hydration_and_clears_on_success() {
    let request = SentenceBatchSettings::new(Some(SentenceLevel::B1), SentenceTypeMix::Varied)
        .selections(1)
        .remove(0)
        .expect("varied settings must create one request");
    let requested = CardDraft::new("canard", "a duck", LanguagePair::new("fr", "en"))
        .requesting_meta(request)
        .with_costs(ArtifactCosts::default());
    let completed = requested.clone().with_meta(
        CardMeta::new(
            "ka.naʁ",
            "lə ka.naʁ naʒ",
            "duck",
            5,
            "The duck swims",
            "duck",
            "Think of a pond",
            "A concrete noun",
            "Le canard nage",
        ),
        None,
    );
    assert_eq!(
        (requested.meta_request().is_some(), completed.meta_request()),
        (true, None),
        "initial metadata request disappeared before generation or survived its successful pass"
    );
}
