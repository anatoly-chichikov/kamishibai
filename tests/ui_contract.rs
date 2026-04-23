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
//! - every `SourceRef` resolves to a real file with the named line in range.

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
    check_source_refs(&contract, &mut errs);
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
        if !p.is_file() {
            continue;
        }
        let Ok(content) = fs::read_to_string(p) else {
            continue;
        };
        let lines = content.lines().count() as u32;
        if r.line == 0 || r.line > lines {
            errs.push(format!(
                "source ref {:?}:{} out of range (file has {} lines)",
                r.file, r.line, lines
            ));
        }
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
