//! Typed intent inventory for the TUI and its structural, copy and evidence checks.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use kamishibai::tui::{BusyKind, ModalKind, Screen};
use serde::Deserialize;

/// Authored interface intent; load with `try_from_document` and inspect its links and copy.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Catalog {
    version: u32,
    verifications: Vec<Verification>,
    rules: Vec<Rule>,
    map: Map,
}

/// One command that checks the TUI, what a green run establishes, and what it leaves unproven.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Verification {
    command: String,
    establishes: String,
    limits: String,
}

/// The state map: every runtime state and the scenarios that route through them.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Map {
    states: Vec<State>,
    scenarios: Vec<Scenario>,
}

/// The one-line head every entry opens with: its stable ID and the intent an edit must preserve.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Head {
    id: String,
    intent: String,
}

/// A shared rule: its head, the expectations it imposes, and what it rests on.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    head: Head,
    expectations: Vec<String>,
    grounds: Grounds,
}

/// What an entry rests on: why it exists, the code that implements it, the tests that check it and the gaps where code disagrees.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Grounds {
    #[serde(default)]
    rationale: Vec<String>,
    sources: Vec<Source>,
    tests: Vec<Check>,
    #[serde(default)]
    gaps: Vec<Gap>,
}

/// Code behavior that contradicts an expectation today; the expectation keeps the intent.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Gap {
    summary: String,
    sources: Vec<Source>,
}

/// One runtime situation: its head, its contract, what it rests on and the captures that show it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    head: Head,
    contract: Contract,
    grounds: Grounds,
    evidence: Evidence,
}

/// What a state promises: where it lives, the chrome it draws, what must hold and where each event leads.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    surface: Surface,
    chrome: Chrome,
    expectations: Vec<String>,
    actions: Vec<Action>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Surface {
    screen: String,
    overlay: Overlay,
}

#[derive(Deserialize)]
enum Overlay {
    Clear,
    Modal(String),
    Busy(String),
    Error,
}

/// The header title, the dim tagline beside it and the footer hints, as the renderer draws them.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Chrome {
    title: String,
    hint: String,
    footer: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    event: String,
    target: String,
    condition: String,
    expectation: String,
}

#[derive(Deserialize)]
enum Evidence {
    Missing(String),
    Captured(Vec<Shot>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Shot {
    file: PathBuf,
    origin: Origin,
    revision: Option<String>,
    note: String,
}

/// Where a capture comes from: a prepared scene, the live tape, or an unestablished origin.
#[derive(Deserialize, PartialEq, Eq)]
enum Origin {
    Synthetic,
    Live,
    Unrecorded,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    file: PathBuf,
    symbol: String,
}

/// A behavior test whose assertions exercise the entry; `test` names the function inside `file`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Check {
    file: PathBuf,
    test: String,
}

/// A route through states with the expectations it must keep and the tests that walk it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    head: Head,
    route: Vec<String>,
    expectations: Vec<String>,
    tests: Vec<Check>,
}

/// An inventory defect; it does not grade design intent beyond the copy it can compare.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Violation {
    Version(u32),
    Empty(String),
    Duplicate(String),
    UnknownSurface(String),
    MissingSurface(String),
    UnknownState(String),
    DisconnectedRoute(String),
    MissingSource(String),
    MissingSymbol(String),
    MissingTest(String),
    InvalidCapture(String),
    UncataloguedCapture(String),
    InvalidRevision(String),
    Provenance(String),
    Header(String),
    Footer(String),
    UnknownCommand(String),
}

impl Catalog {
    /// Decode the authored RON document without accepting unknown fields.
    pub(super) fn try_from_document(document: &str) -> Result<Self> {
        ron::from_str(document).context("Cannot decode TUI intent catalog")
    }

    /// Check that every real capture fixture has an authored inventory state.
    #[must_use]
    pub(super) fn inspect_scenes(&self, names: &[String]) -> Vec<Violation> {
        names
            .iter()
            .filter(|name| !self.map.states.iter().any(|state| &state.head.id == *name))
            .map(|name| Violation::UnknownState(name.clone()))
            .collect()
    }

    /// Compare each prepared scene's rendered header and footer with its catalog state.
    ///
    /// `renders` pairs a fixture ID with the plain text the real renderer drew for it.
    #[must_use]
    pub(super) fn inspect_renders(&self, renders: &[(String, String)]) -> Vec<Violation> {
        let mut faults = Vec::new();
        for (id, screen) in renders {
            let Some(state) = self.map.states.iter().find(|state| &state.head.id == id) else {
                faults.push(Violation::UnknownState(id.clone()));
                continue;
            };
            let chrome = &state.contract.chrome;
            let drawn = tagline(screen, &chrome.title);
            if drawn != Some(chrome.hint.as_str()) {
                faults.push(Violation::Header(format!(
                    "{id}: catalog {:?} / {:?} but renderer drew {drawn:?}",
                    chrome.title, chrome.hint
                )));
            }
            let footer = hints(screen.lines().last().unwrap_or_default());
            if footer != chrome.footer {
                faults.push(Violation::Footer(format!(
                    "{id}: catalog {:?} but renderer drew {footer:?}",
                    chrome.footer
                )));
            }
        }
        faults
    }

    /// Inspect shape, surfaces, routes, sources, tests, PNG links and tape provenance.
    #[must_use]
    pub(super) fn inspect(&self, root: &Path) -> Vec<Violation> {
        let mut faults = Vec::new();
        if self.version != 2 {
            faults.push(Violation::Version(self.version));
        }
        verifications(&self.verifications, root, &mut faults);
        let states = &self.map.states;
        identifiers(&self.rules, |rule| &rule.head.id, "rules", &mut faults);
        identifiers(states, |state| &state.head.id, "states", &mut faults);
        identifiers(
            &self.map.scenarios,
            |scenario| &scenario.head.id,
            "scenarios",
            &mut faults,
        );
        for rule in &self.rules {
            text(&rule.head.intent, &rule.head.id, &mut faults);
            expectations(&rule.expectations, &rule.head.id, &mut faults);
            grounds(&rule.grounds, root, &rule.head.id, &mut faults);
        }
        let ids = states
            .iter()
            .map(|state| state.head.id.as_str())
            .collect::<HashSet<_>>();
        for state in states {
            let owner = &state.head.id;
            let contract = &state.contract;
            text(&state.head.intent, owner, &mut faults);
            surface(&contract.surface, owner, &mut faults);
            text(&contract.chrome.title, owner, &mut faults);
            text(&contract.chrome.hint, owner, &mut faults);
            for hint in &contract.chrome.footer {
                text(hint, owner, &mut faults);
            }
            expectations(&contract.expectations, owner, &mut faults);
            grounds(&state.grounds, root, owner, &mut faults);
            captures(&state.evidence, root, owner, &mut faults);
            for action in &contract.actions {
                text(&action.event, owner, &mut faults);
                text(&action.condition, owner, &mut faults);
                text(&action.expectation, owner, &mut faults);
                if !ids.contains(action.target.as_str()) {
                    faults.push(Violation::UnknownState(format!(
                        "{owner} -> {}",
                        action.target
                    )));
                }
            }
        }
        for scenario in &self.map.scenarios {
            let owner = &scenario.head.id;
            text(&scenario.head.intent, owner, &mut faults);
            expectations(&scenario.expectations, owner, &mut faults);
            checks(&scenario.tests, root, owner, &mut faults);
            if scenario.route.len() < 2 {
                faults.push(Violation::Empty(format!("{owner} route")));
            }
            for id in &scenario.route {
                if !ids.contains(id.as_str()) {
                    faults.push(Violation::UnknownState(format!("{owner}: {id}")));
                }
            }
            for pair in scenario.route.windows(2) {
                let linked = states.iter().any(|state| {
                    state.head.id == pair[0]
                        && state
                            .contract
                            .actions
                            .iter()
                            .any(|action| action.target == pair[1])
                });
                if !linked {
                    faults.push(Violation::DisconnectedRoute(format!(
                        "{owner}: {} -> {}",
                        pair[0], pair[1]
                    )));
                }
            }
        }
        coverage(states, &mut faults);
        inventory_captures(states, root, &mut faults);
        provenance(states, root, &mut faults);
        faults
    }
}

fn tagline<'a>(screen: &'a str, title: &str) -> Option<&'a str> {
    screen.lines().find_map(|line| {
        let start = line.find(title)?;
        line[start + title.len()..].trim_start().split("  ").next()
    })
}

fn hints(line: &str) -> Vec<String> {
    line.find('[').map_or_else(Vec::new, |start| {
        line[start..]
            .split("  ")
            .map(str::trim)
            .filter(|hint| !hint.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

fn identifiers<T>(
    items: &[T],
    id: impl Fn(&T) -> &String,
    owner: &str,
    faults: &mut Vec<Violation>,
) {
    let mut known = HashSet::new();
    if items.is_empty() {
        faults.push(Violation::Empty(owner.to_owned()));
    }
    for item in items {
        let value = id(item);
        text(value, owner, faults);
        if !known.insert(value) {
            faults.push(Violation::Duplicate(format!("{owner}: {value}")));
        }
    }
}

fn text(value: &str, owner: &str, faults: &mut Vec<Violation>) {
    if value.trim().is_empty() {
        faults.push(Violation::Empty(owner.to_owned()));
    }
}

fn expectations(values: &[String], owner: &str, faults: &mut Vec<Violation>) {
    if values.is_empty() {
        faults.push(Violation::Empty(format!("{owner} expectations")));
    }
    for value in values {
        text(value, owner, faults);
    }
}

fn grounds(value: &Grounds, root: &Path, owner: &str, faults: &mut Vec<Violation>) {
    for reason in &value.rationale {
        text(reason, owner, faults);
    }
    sources(&value.sources, root, owner, faults);
    checks(&value.tests, root, owner, faults);
    gaps(&value.gaps, root, owner, faults);
}

fn surface(value: &Surface, owner: &str, faults: &mut Vec<Violation>) {
    if !screens()
        .iter()
        .any(|screen| screen_name(*screen) == value.screen)
    {
        faults.push(Violation::UnknownSurface(format!(
            "{owner}: {}",
            value.screen
        )));
    }
    let known = match &value.overlay {
        Overlay::Clear | Overlay::Error => true,
        Overlay::Modal(name) => modals().iter().any(|modal| modal_name(*modal) == name),
        Overlay::Busy(name) => jobs().iter().any(|job| busy_name(*job) == name),
    };
    if !known {
        faults.push(Violation::UnknownSurface(owner.to_owned()));
    }
}

fn sources(values: &[Source], root: &Path, owner: &str, faults: &mut Vec<Violation>) {
    if values.is_empty() {
        faults.push(Violation::Empty(format!("{owner} sources")));
    }
    for source in values {
        let Some(document) = readable(root, &source.file) else {
            faults.push(Violation::MissingSource(format!(
                "{owner}: {}",
                source.file.display()
            )));
            continue;
        };
        let valid = !source.symbol.is_empty()
            && source
                .symbol
                .chars()
                .all(|symbol| symbol.is_ascii_alphanumeric() || symbol == '_');
        let present = valid
            && document
                .split(|symbol: char| !symbol.is_ascii_alphanumeric() && symbol != '_')
                .any(|token| token == source.symbol);
        if !present {
            faults.push(Violation::MissingSymbol(format!(
                "{owner}: {}::{}",
                source.file.display(),
                source.symbol
            )));
        }
    }
}

fn checks(values: &[Check], root: &Path, owner: &str, faults: &mut Vec<Violation>) {
    for check in values {
        let defined = readable(root, &check.file).is_some_and(|document| {
            !check.test.is_empty()
                && check
                    .test
                    .chars()
                    .all(|symbol| symbol.is_ascii_alphanumeric() || symbol == '_')
                && document.contains(&format!("fn {}(", check.test))
        });
        if !defined {
            faults.push(Violation::MissingTest(format!(
                "{owner}: {}::{}",
                check.file.display(),
                check.test
            )));
        }
    }
}

fn gaps(values: &[Gap], root: &Path, owner: &str, faults: &mut Vec<Violation>) {
    for gap in values {
        text(&gap.summary, owner, faults);
        sources(&gap.sources, root, owner, faults);
    }
}

fn verifications(values: &[Verification], root: &Path, faults: &mut Vec<Violation>) {
    if values.is_empty() {
        faults.push(Violation::Empty(String::from("verifications")));
    }
    let manifest = readable(root, Path::new("Cargo.toml")).unwrap_or_default();
    let harness = readable(root, Path::new("tests/all.rs")).unwrap_or_default();
    for verification in values {
        text(&verification.command, "verifications", faults);
        text(&verification.establishes, &verification.command, faults);
        text(&verification.limits, &verification.command, faults);
        let words = verification.command.split_whitespace().collect::<Vec<_>>();
        for pair in words.windows(2) {
            if pair[0] == "--test" && !manifest.contains(&format!("name = \"{}\"", pair[1])) {
                faults.push(Violation::UnknownCommand(verification.command.clone()));
            }
        }
        for filter in words.iter().filter_map(|word| word.strip_suffix("::")) {
            if !harness.contains(&format!("mod {filter};")) {
                faults.push(Violation::UnknownCommand(verification.command.clone()));
            }
        }
    }
}

fn readable(root: &Path, file: &Path) -> Option<String> {
    if !relative(file) {
        return None;
    }
    fs::read_to_string(root.join(file)).ok()
}

fn captures(value: &Evidence, root: &Path, owner: &str, faults: &mut Vec<Violation>) {
    match value {
        Evidence::Missing(reason) => text(reason, owner, faults),
        Evidence::Captured(shots) => {
            if shots.is_empty() {
                faults.push(Violation::Empty(format!("{owner} captures")));
            }
            for shot in shots {
                let path = root.join(&shot.file);
                let valid = relative(&shot.file)
                    && shot
                        .file
                        .extension()
                        .is_some_and(|extension| extension == "png")
                    && image::open(path).is_ok();
                if !valid {
                    faults.push(Violation::InvalidCapture(format!(
                        "{owner}: {}",
                        shot.file.display()
                    )));
                }
                text(&shot.note, owner, faults);
                if let Some(revision) = &shot.revision
                    && (!(7..=64).contains(&revision.len())
                        || !revision.chars().all(|symbol| symbol.is_ascii_hexdigit()))
                {
                    faults.push(Violation::InvalidRevision(format!("{owner}: {revision}")));
                }
            }
        }
    }
}

fn relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn coverage(states: &[State], faults: &mut Vec<Violation>) {
    for screen in screens() {
        let name = screen_name(screen);
        if !states
            .iter()
            .any(|state| state.contract.surface.screen == name)
        {
            faults.push(Violation::MissingSurface(name.to_owned()));
        }
    }
    for modal in modals() {
        let name = modal_name(modal);
        if !states.iter().any(|state| {
            matches!(&state.contract.surface.overlay, Overlay::Modal(value) if value == name)
        }) {
            faults.push(Violation::MissingSurface(name.to_owned()));
        }
    }
    for job in jobs() {
        let name = busy_name(job);
        if !states.iter().any(|state| {
            matches!(&state.contract.surface.overlay, Overlay::Busy(value) if value == name)
        }) {
            faults.push(Violation::MissingSurface(name.to_owned()));
        }
    }
    if !states
        .iter()
        .any(|state| matches!(state.contract.surface.overlay, Overlay::Error))
    {
        faults.push(Violation::MissingSurface(String::from("Error")));
    }
}

fn inventory_captures(states: &[State], root: &Path, faults: &mut Vec<Violation>) {
    let directory = root.join("docs/tui-states/live");
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let known = states
        .iter()
        .flat_map(|state| match &state.evidence {
            Evidence::Missing(_) => Vec::new(),
            Evidence::Captured(shots) => shots.iter().map(|shot| root.join(&shot.file)).collect(),
        })
        .collect::<HashSet<_>>();
    for entry in entries {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if path.extension().is_some_and(|extension| extension == "png")
                    && path.file_name().is_some_and(|name| name != "caption.png")
                    && !known.contains(&path)
                {
                    faults.push(Violation::UncataloguedCapture(path.display().to_string()));
                }
            }
            Err(error) => faults.push(Violation::InvalidCapture(error.to_string())),
        }
    }
}

fn provenance(states: &[State], root: &Path, faults: &mut Vec<Violation>) {
    let synthetic = ["states.tape", "states-narrow.tape"]
        .into_iter()
        .flat_map(|tape| screenshots(root, tape, faults))
        .collect::<Vec<_>>();
    let live = screenshots(root, "capture.tape", faults);
    for state in states {
        let Evidence::Captured(shots) = &state.evidence else {
            continue;
        };
        for shot in shots {
            let written = |list: &[(Option<String>, PathBuf)]| {
                list.iter().any(|(scene, file)| {
                    *file == shot.file
                        && (shot.origin != Origin::Synthetic
                            || scene.as_deref() == Some(state.head.id.as_str()))
                })
            };
            let backed = match shot.origin {
                Origin::Synthetic => written(&synthetic),
                Origin::Live => written(&live),
                Origin::Unrecorded => !written(&synthetic) && !written(&live),
            };
            if !backed {
                faults.push(Violation::Provenance(format!(
                    "{}: {} is not written as a {} capture of this state",
                    state.head.id,
                    shot.file.display(),
                    match shot.origin {
                        Origin::Synthetic => "synthetic",
                        Origin::Live => "live",
                        Origin::Unrecorded => "unrecorded",
                    }
                )));
            }
        }
    }
    let owned = |origin: Origin, scene: Option<&str>, file: &Path| {
        states.iter().any(|state| {
            scene.is_none_or(|scene| scene == state.head.id) && captured(state, &origin, file)
        })
    };
    for (scene, file) in &synthetic {
        if scene.is_none() || !owned(Origin::Synthetic, scene.as_deref(), file) {
            faults.push(Violation::Provenance(format!(
                "a synthetic tape writes {} from {scene:?} without a matching catalog capture",
                file.display()
            )));
        }
    }
    for (_, file) in &live {
        if !owned(Origin::Live, None, file) {
            faults.push(Violation::Provenance(format!(
                "capture.tape writes {} without a matching live catalog capture",
                file.display()
            )));
        }
    }
}

fn captured(state: &State, origin: &Origin, file: &Path) -> bool {
    match &state.evidence {
        Evidence::Missing(_) => false,
        Evidence::Captured(shots) => shots
            .iter()
            .any(|shot| shot.origin == *origin && shot.file == file),
    }
}

fn screenshots(
    root: &Path,
    tape: &str,
    faults: &mut Vec<Violation>,
) -> Vec<(Option<String>, PathBuf)> {
    let directory = Path::new("docs/tui-states");
    let Ok(document) = fs::read_to_string(root.join(directory).join(tape)) else {
        faults.push(Violation::Provenance(format!("{tape} is unreadable")));
        return Vec::new();
    };
    let mut scene: Option<String> = None;
    let mut shots = Vec::new();
    for line in document.lines() {
        if let Some((_, rest)) = line.split_once("--state ") {
            scene = rest
                .split(|symbol: char| symbol == '"' || symbol.is_whitespace())
                .next()
                .map(str::to_owned);
        }
        if let Some(file) = line
            .trim()
            .strip_prefix("Screenshot \"")
            .and_then(|rest| rest.strip_suffix('"'))
        {
            shots.push((scene.clone(), directory.join(file)));
        }
    }
    shots
}

fn screens() -> [Screen; 5] {
    [
        Screen::Welcome,
        Screen::YourWords,
        Screen::WhatIUnderstood,
        Screen::YourCards,
        Screen::Done,
    ]
}

fn screen_name(screen: Screen) -> &'static str {
    match screen {
        Screen::Welcome => "Welcome",
        Screen::YourWords => "YourWords",
        Screen::WhatIUnderstood => "WhatIUnderstood",
        Screen::YourCards => "YourCards",
        Screen::Done => "Done",
    }
}

fn modals() -> [ModalKind; 3] {
    [
        ModalKind::ChangeSomething,
        ModalKind::PickLanguages,
        ModalKind::PickTranslationLanguage,
    ]
}

fn modal_name(modal: ModalKind) -> &'static str {
    match modal {
        ModalKind::ChangeSomething => "ChangeSomething",
        ModalKind::PickLanguages => "PickLanguages",
        ModalKind::PickTranslationLanguage => "PickTranslationLanguage",
    }
}

fn jobs() -> [BusyKind; 5] {
    [
        BusyKind::Understanding,
        BusyKind::BulkCorrection,
        BusyKind::CheckingKey,
        BusyKind::PublishingDeck,
        BusyKind::PublishingReport,
    ]
}

fn busy_name(job: BusyKind) -> &'static str {
    match job {
        BusyKind::Understanding => "Understanding",
        BusyKind::BulkCorrection => "BulkCorrection",
        BusyKind::CheckingKey => "CheckingKey",
        BusyKind::PublishingDeck => "PublishingDeck",
        BusyKind::PublishingReport => "PublishingReport",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        Action, Catalog, Check, Chrome, Contract, Evidence, Gap, Grounds, Head, Map, Origin,
        Overlay, Scenario, Shot, Source, State, Surface, Verification, Violation,
    };

    fn head(id: &str, intent: &str) -> Head {
        Head {
            id: id.to_owned(),
            intent: intent.to_owned(),
        }
    }

    fn state(id: &str) -> State {
        State {
            head: head(id, "Inspect a prepared card"),
            contract: Contract {
                surface: Surface {
                    screen: String::from("YourCards"),
                    overlay: Overlay::Clear,
                },
                chrome: Chrome {
                    title: String::from("your cards"),
                    hint: String::from("open a card to follow progress"),
                    footer: vec![String::from("[Esc] stop"), String::from("[Ctrl+C] quit")],
                },
                expectations: vec![String::from("Focus stays on its head")],
                actions: Vec::new(),
            },
            grounds: Grounds {
                rationale: Vec::new(),
                sources: vec![Source {
                    file: "render.rs".into(),
                    symbol: String::from("render"),
                }],
                tests: Vec::new(),
                gaps: Vec::new(),
            },
            evidence: Evidence::Missing(String::from("No capture recorded")),
        }
    }

    fn catalog(states: Vec<State>, scenarios: Vec<Scenario>) -> Catalog {
        Catalog {
            version: 2,
            verifications: Vec::new(),
            rules: Vec::new(),
            map: Map { states, scenarios },
        }
    }

    fn screen(header: &str, footer: &str) -> String {
        format!(
            "\n     your cards   {header}          EN → FR\n\n    step 3/3  2/4 ready    {footer}"
        )
    }

    #[test]
    fn catalog_cannot_accept_duplicate_state_names() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        assert!(
            catalog(
                vec![state("cards.closed"), state("cards.closed")],
                Vec::new()
            )
            .inspect(directory.path())
            .iter()
            .any(|fault| matches!(fault, Violation::Duplicate(_))),
            "catalog accepted duplicate state names"
        );
    }

    #[test]
    fn capture_fixtures_cannot_bypass_the_intent_inventory() {
        assert!(
            catalog(vec![state("cards.open")], Vec::new())
                .inspect_scenes(&[String::from("cards.uncatalogued")])
                .iter()
                .any(|fault| matches!(fault, Violation::UnknownState(_))),
            "capture fixture escaped the intent inventory"
        );
    }

    #[test]
    fn catalog_cannot_link_to_an_unknown_state() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        let mut entry = state("cards.closed");
        entry.contract.actions.push(Action {
            event: String::from("Enter"),
            target: String::from("cards.disappeared"),
            condition: String::from("A card is focused"),
            expectation: String::from("Open that card"),
        });
        assert!(
            catalog(vec![entry], Vec::new())
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::UnknownState(_))),
            "catalog accepted a dangling transition"
        );
    }

    #[test]
    fn catalog_cannot_accept_a_route_without_a_transition() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        let scenario = Scenario {
            head: head("cards.inspect", "Read a card"),
            route: vec![String::from("cards.closed"), String::from("cards.open")],
            expectations: vec![String::from("Keep the head focused")],
            tests: Vec::new(),
        };
        assert!(
            catalog(
                vec![state("cards.closed"), state("cards.open")],
                vec![scenario]
            )
            .inspect(directory.path())
            .iter()
            .any(|fault| matches!(fault, Violation::DisconnectedRoute(_))),
            "catalog accepted an unconnected scenario route"
        );
    }

    #[test]
    fn catalog_cannot_treat_a_file_as_proof_of_a_missing_symbol() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        fs::write(directory.path().join("render.rs"), "fn painted() {}\n")
            .expect("invariant: source fixture must be writable");
        assert!(
            catalog(vec![state("cards.open")], Vec::new())
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::MissingSymbol(_))),
            "catalog accepted a stale source symbol"
        );
    }

    #[test]
    fn catalog_cannot_cite_a_test_that_does_not_exist() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        fs::write(
            directory.path().join("cards.rs"),
            "#[test]\nfn opening_keeps_the_head_focused() {}\n",
        )
        .expect("invariant: test fixture must be writable");
        let mut entry = state("cards.open");
        entry.grounds.tests.push(Check {
            file: "cards.rs".into(),
            test: String::from("opening_lights_the_editor"),
        });
        assert!(
            catalog(vec![entry], Vec::new())
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::MissingTest(_))),
            "catalog cited a behavior test that does not exist"
        );
    }

    #[test]
    fn catalog_cannot_treat_a_named_png_as_a_readable_capture() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        fs::write(directory.path().join("broken.png"), "not an image")
            .expect("invariant: image fixture must be writable");
        let mut entry = state("cards.open");
        entry.evidence = Evidence::Captured(vec![Shot {
            file: "broken.png".into(),
            origin: Origin::Unrecorded,
            revision: None,
            note: String::from("Historical capture"),
        }]);
        assert!(
            catalog(vec![entry], Vec::new())
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::InvalidCapture(_))),
            "catalog accepted corrupt screenshot bytes"
        );
    }

    #[test]
    fn catalog_cannot_omit_runtime_overlays() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        let faults = catalog(vec![state("cards.open")], Vec::new()).inspect(directory.path());
        assert!(
            faults.contains(&Violation::MissingSurface(String::from(
                "PickTranslationLanguage"
            ))),
            "catalog silently omitted the translation picker"
        );
    }

    #[test]
    fn catalog_cannot_publish_an_unlisted_capture() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        let captures = directory.path().join("docs/tui-states/live");
        fs::create_dir_all(&captures).expect("invariant: capture directory must be writable");
        image::RgbImage::new(7, 3)
            .save(captures.join("unknown.png"))
            .expect("invariant: image fixture must encode");
        assert!(
            catalog(vec![state("cards.open")], Vec::new())
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::UncataloguedCapture(_))),
            "catalog allowed an unexplained screenshot"
        );
    }

    #[test]
    fn catalog_cannot_accept_a_source_outside_the_repository() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        let mut entry = state("cards.open");
        entry.grounds.sources[0].file = directory.path().join("render.rs");
        fs::write(&entry.grounds.sources[0].file, "fn render() {}\n")
            .expect("invariant: source fixture must be writable");
        assert!(
            catalog(vec![entry], Vec::new())
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::MissingSource(_))),
            "catalog accepted an absolute source path"
        );
    }

    #[test]
    fn synthetic_capture_cannot_come_from_another_scene() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        let tapes = directory.path().join("docs/tui-states");
        fs::create_dir_all(tapes.join("live")).expect("invariant: tape directory must exist");
        fs::write(
            tapes.join("states.tape"),
            "Type \"./tui_states --state cards.closed\"\nScreenshot \"live/open.png\"\n",
        )
        .expect("invariant: tape fixture must be writable");
        for tape in ["states-narrow.tape", "capture.tape"] {
            fs::write(tapes.join(tape), "").expect("invariant: tape fixture must be writable");
        }
        image::RgbImage::new(7, 3)
            .save(tapes.join("live/open.png"))
            .expect("invariant: image fixture must encode");
        let mut entry = state("cards.open");
        entry.evidence = Evidence::Captured(vec![Shot {
            file: "docs/tui-states/live/open.png".into(),
            origin: Origin::Synthetic,
            revision: None,
            note: String::from("Fresh capture"),
        }]);
        assert!(
            catalog(vec![entry, state("cards.closed")], Vec::new())
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::Provenance(_))),
            "a capture of another scene was accepted as evidence for this state"
        );
    }

    #[test]
    fn catalog_cannot_claim_a_footer_the_renderer_does_not_draw() {
        assert!(
            catalog(vec![state("cards.generating")], Vec::new())
                .inspect_renders(&[(
                    String::from("cards.generating"),
                    screen(
                        "open a card to follow progress",
                        "[Esc] again  [Ctrl+C] quit"
                    ),
                )])
                .iter()
                .any(|fault| matches!(fault, Violation::Footer(_))),
            "catalog footer survived a renderer that drew different hints"
        );
    }

    #[test]
    fn catalog_cannot_claim_a_header_the_renderer_does_not_draw() {
        assert!(
            catalog(vec![state("cards.generating")], Vec::new())
                .inspect_renders(&[(
                    String::from("cards.generating"),
                    screen("your files are ready", "[Esc] stop  [Ctrl+C] quit"),
                )])
                .iter()
                .any(|fault| matches!(fault, Violation::Header(_))),
            "catalog header survived a renderer that drew a different tagline"
        );
    }

    #[test]
    fn matching_render_cannot_raise_a_copy_fault() {
        assert!(
            catalog(vec![state("cards.generating")], Vec::new())
                .inspect_renders(&[(
                    String::from("cards.generating"),
                    screen(
                        "open a card to follow progress",
                        "[Esc] stop  [Ctrl+C] quit"
                    ),
                )])
                .is_empty(),
            "an exact render was reported as drifted copy"
        );
    }

    #[test]
    fn catalog_cannot_cite_a_gap_in_a_missing_symbol() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        fs::write(directory.path().join("render.rs"), "fn render() {}\n")
            .expect("invariant: source fixture must be writable");
        let mut entry = state("cards.open");
        entry.grounds.gaps.push(Gap {
            summary: String::from("Escape stop is advertised over errors"),
            sources: vec![Source {
                file: "render.rs".into(),
                symbol: String::from("screen_hints"),
            }],
        });
        assert!(
            catalog(vec![entry], Vec::new())
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::MissingSymbol(_))),
            "a known gap pointed at code that does not exist"
        );
    }

    #[test]
    fn verification_cannot_name_an_undeclared_test_module() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        fs::write(
            directory.path().join("Cargo.toml"),
            "[[test]]\nname = \"all\"\n",
        )
        .expect("invariant: manifest fixture must be writable");
        fs::create_dir_all(directory.path().join("tests"))
            .expect("invariant: test directory must be writable");
        fs::write(directory.path().join("tests/all.rs"), "mod ui_catalog;\n")
            .expect("invariant: harness fixture must be writable");
        let mut inventory = catalog(vec![state("cards.open")], Vec::new());
        inventory.verifications.push(Verification {
            command: String::from("cargo test --test all ui_contract::"),
            establishes: String::from("The retired contract parses"),
            limits: String::from("It never executes a transition"),
        });
        assert!(
            inventory
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::UnknownCommand(_))),
            "a verification named a test module the harness does not declare"
        );
    }
}
