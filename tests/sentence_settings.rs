use kamishibai::session::{
    ArtifactCosts, CardDraft, CardMeta, LanguagePair, SentenceAxis, SentenceBatchSettings,
    SentenceKind, SentenceLevel, SentenceTypeMix,
};

#[test]
fn best_fit_defaults_add_no_initial_label_request() {
    assert_eq!(
        SentenceBatchSettings::default().selections(7),
        vec![None; 7],
        "best-fit sentence settings introduced a pinned generation request"
    );
}

#[test]
fn a_batch_level_is_pinned_on_every_card() {
    let selections =
        SentenceBatchSettings::new(Some(SentenceLevel::B1), SentenceTypeMix::BestFit).selections(7);
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
fn exact_type_modes_pin_every_final_draft() {
    let modes = [
        (SentenceTypeMix::Statements, SentenceKind::Statement),
        (SentenceTypeMix::Questions, SentenceKind::Question),
        (SentenceTypeMix::Dialogue, SentenceKind::Dialogue),
    ];
    let allocated = modes
        .into_iter()
        .map(|(mode, expected)| {
            let pinned = SentenceBatchSettings::new(None, mode)
                .selections(7)
                .into_iter()
                .all(|selection| {
                    let selection = selection.expect("an exact type mode must create a request");
                    selection.kind() == Some(expected)
                        && selection.pinned().contains(SentenceAxis::Type)
                });
            (mode.token(), mode.pins(), pinned)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        allocated,
        vec![
            ("statements", true, true),
            ("questions", true, true),
            ("dialogue", true, true),
        ],
        "an exact sentence-type mode failed to pin every final draft"
    );
}

#[test]
fn mixed_types_are_deterministic_even_and_never_forced() {
    let settings = SentenceBatchSettings::new(None, SentenceTypeMix::Mixed);
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
        (true, 34, 33, 33, true),
        "mixed allocation was unstable, unweighted, or used a forced sentence type"
    );
}

#[test]
fn old_type_policy_tokens_read_as_their_new_canonical_modes() {
    let natural: SentenceTypeMix =
        serde_json::from_str("\"natural\"").expect("legacy natural token must decode");
    let varied: SentenceTypeMix =
        serde_json::from_str("\"varied\"").expect("legacy varied token must decode");
    assert_eq!(
        (
            natural,
            varied,
            serde_json::to_string(&natural).expect("best-fit mode must encode"),
            serde_json::to_string(&varied).expect("mixed mode must encode"),
            natural.pins(),
            varied.pins(),
        ),
        (
            SentenceTypeMix::BestFit,
            SentenceTypeMix::Mixed,
            String::from("\"best-fit\""),
            String::from("\"mixed\""),
            false,
            true,
        ),
        "legacy sentence-type policies failed to migrate to canonical tokens"
    );
}

#[test]
fn an_initial_meta_request_survives_hydration_and_clears_on_success() {
    let request = SentenceBatchSettings::new(Some(SentenceLevel::B1), SentenceTypeMix::Mixed)
        .selections(1)
        .remove(0)
        .expect("mixed settings must create one request");
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
