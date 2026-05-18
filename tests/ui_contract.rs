//! Typed schema + invariants for `docs/tui-states/ui-contract.ron`.
//!
//! The RON file is a serialized instance of `schema::Contract`. The single
//! `loads_ui_contract` test deserializes it (which alone validates every
//! `enum` variant, the `Score` 1..=5 range, and field shape), then runs a
//! handful of cross-element invariants the type system cannot express:
//!
//! - all four locked-in `ScreenId`s appear exactly once;
//! - every `Issue::DuplicateOf::other` points at an `ElementId` that exists;
//! - every `Element` with `health: Broken | Fake` carries at least one `Issue`;
//! - every `Element` with open issues has degraded health;
//! - every `SourceRef` resolves to a real file with the named line in range.
//! - the `YourWords` input contract stays line-delimited and does not bind plain `Enter`.

#![allow(dead_code)]

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use schema::*;

const CONTRACT_PATH: &str = "docs/tui-states/ui-contract.ron";

const ALL_SCREEN_IDS: &[ScreenId] = &[
    ScreenId::YourWords,
    ScreenId::WhatIUnderstood,
    ScreenId::YourCards,
    ScreenId::Done,
];

#[test]
fn loads_ui_contract() {
    let text =
        fs::read_to_string(CONTRACT_PATH).unwrap_or_else(|e| panic!("read {CONTRACT_PATH}: {e}"));
    let contract: Contract =
        ron::from_str(&text).unwrap_or_else(|e| panic!("parse {CONTRACT_PATH}: {e}"));
    let mut errs: Vec<String> = Vec::new();
    check_screens_exhaustive(&contract, &mut errs);
    let known_ids = collect_element_ids(&contract);
    check_duplicate_targets(&contract, &known_ids, &mut errs);
    check_broken_or_fake_have_issues(&contract, &mut errs);
    check_issueful_elements_are_degraded(&contract, &mut errs);
    check_source_refs(&contract, &mut errs);
    check_your_words_input_contract(&contract, &mut errs);
    assert!(
        errs.is_empty(),
        "ui-contract failed {} invariant(s):\n  - {}",
        errs.len(),
        errs.join("\n  - ")
    );
}

fn check_screens_exhaustive(contract: &Contract, errs: &mut Vec<String>) {
    for sid in ALL_SCREEN_IDS {
        let count = contract.app.screens.iter().filter(|s| &s.id == sid).count();
        if count != 1 {
            errs.push(format!(
                "screen {sid:?} appears {count} time(s); expected 1"
            ));
        }
    }
    if contract.app.screens.len() != ALL_SCREEN_IDS.len() {
        errs.push(format!(
            "expected {} screens, found {}",
            ALL_SCREEN_IDS.len(),
            contract.app.screens.len()
        ));
    }
}

fn collect_element_ids(contract: &Contract) -> HashSet<ElementId> {
    let mut out = HashSet::new();
    visit_elements(contract, |e| {
        if !out.insert(e.id.clone()) {
            // duplicate ids are caught separately, this just keeps the set tight
        }
    });
    out
}

fn check_duplicate_targets(
    contract: &Contract,
    known: &HashSet<ElementId>,
    errs: &mut Vec<String>,
) {
    visit_elements(contract, |elem| {
        for issue in &elem.issues {
            if let Issue::DuplicateOf { other, .. } = issue
                && !known.contains(other)
            {
                errs.push(format!(
                    "{:?} -> Issue::DuplicateOf points at unknown id {:?}",
                    elem.id, other
                ));
            }
        }
    });
}

fn check_broken_or_fake_have_issues(contract: &Contract, errs: &mut Vec<String>) {
    visit_elements(contract, |e| {
        let needs = matches!(e.health, Health::Broken | Health::Fake);
        if needs && e.issues.is_empty() {
            errs.push(format!("{:?} health={:?} but issues=[]", e.id, e.health));
        }
    });
}

fn check_issueful_elements_are_degraded(contract: &Contract, errs: &mut Vec<String>) {
    visit_elements(contract, |e| {
        if !e.issues.is_empty() && matches!(e.health, Health::Working | Health::Decorative) {
            errs.push(format!(
                "{:?} has open issues but health={:?}",
                e.id, e.health
            ));
        }
    });
}

fn check_source_refs(contract: &Contract, errs: &mut Vec<String>) {
    let mut refs: Vec<SourceRef> = Vec::new();
    visit_elements(contract, |e| {
        refs.push(e.source.clone());
        for issue in &e.issues {
            collect_issue_source_refs(issue, &mut refs);
        }
    });
    for r in &refs {
        let p = Path::new(&r.file);
        if !p.exists() {
            errs.push(format!("source file {:?} does not exist", r.file));
            continue;
        }
    }
}

fn check_your_words_input_contract(contract: &Contract, errs: &mut Vec<String>) {
    let Some(placeholder) = element_by_id(contract, "yw.placeholder") else {
        errs.push(String::from("yw.placeholder is missing"));
        return;
    };
    if text_contains(placeholder, "comma") {
        errs.push(String::from("yw.placeholder still advertises comma input"));
    }
    let Some(paste) = element_by_id(contract, "yw.footer_paste") else {
        errs.push(String::from("yw.footer_paste is missing"));
        return;
    };
    if !text_contains(paste, "one per line") {
        errs.push(String::from(
            "yw.footer_paste does not lock line-delimited input",
        ));
    }
    let Some(continue_hint) = element_by_id(contract, "yw.footer_continue") else {
        errs.push(String::from("yw.footer_continue is missing"));
        return;
    };
    if !text_contains(continue_hint, "Ctrl+G") {
        errs.push(String::from("yw.footer_continue does not use Ctrl+G"));
    }
    if text_contains(continue_hint, "[Enter] continue") {
        errs.push(String::from("yw.footer_continue still binds plain Enter"));
    }
}

fn element_by_id<'a>(contract: &'a Contract, id: &str) -> Option<&'a Element> {
    for e in &contract.app.chrome.elements {
        if e.id.0 == id {
            return Some(e);
        }
    }
    for s in &contract.app.screens {
        for r in &s.regions {
            for e in &r.elements {
                if e.id.0 == id {
                    return Some(e);
                }
            }
        }
    }
    for m in &contract.app.modals {
        for r in &m.regions {
            for e in &r.elements {
                if e.id.0 == id {
                    return Some(e);
                }
            }
        }
    }
    None
}

fn text_contains(element: &Element, needle: &str) -> bool {
    match &element.text {
        TextSpec::Literal(text) => text.contains(needle),
        TextSpec::Template(parts) => parts.iter().any(|part| match part {
            TextFragment::Static(text) => text.contains(needle),
            TextFragment::Bind(path) => path.0.contains(needle),
        }),
    }
}

fn collect_issue_source_refs(issue: &Issue, out: &mut Vec<SourceRef>) {
    match issue {
        Issue::BrokenWiring {
            handler_at: Some(s),
            ..
        } => out.push(s.clone()),
        Issue::HiddenBinding { handler_at, .. } => out.push(handler_at.clone()),
        Issue::SpecPromiseUnfulfilled { spec_ref, .. } => out.push(spec_ref.clone()),
        _ => {}
    }
}

fn visit_elements<F: FnMut(&Element)>(contract: &Contract, mut f: F) {
    for e in &contract.app.chrome.elements {
        f(e);
    }
    for s in &contract.app.screens {
        for r in &s.regions {
            for e in &r.elements {
                f(e);
            }
        }
    }
    for m in &contract.app.modals {
        for r in &m.regions {
            for e in &r.elements {
                f(e);
            }
        }
    }
}

#[path = "common/ui_contract_schema.rs"]
mod schema;
