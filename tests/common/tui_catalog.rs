//! Typed intent inventory for the TUI and its structural, copy and evidence checks.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use kamishibai::tui::{self, BusyKind, ModalKind};
use serde::{Deserialize, Deserializer};

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

/// The state map: the screens, the learner's states on them, the fixtures that only check layout and the scenarios through them.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Map {
    screens: Vec<Screen>,
    states: Vec<State>,
    #[serde(deserialize_with = "fixtures")]
    fixtures: Vec<State>,
    scenarios: Vec<Scenario>,
}

/// A prepared scene that exists only to check layout: no learner action leads into it and its intent speaks to developers.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    head: Head,
    contract: Contract,
    grounds: Grounds,
    evidence: Evidence,
}

impl From<Fixture> for State {
    fn from(fixture: Fixture) -> Self {
        Self {
            head: fixture.head,
            contract: fixture.contract,
            grounds: fixture.grounds,
            evidence: fixture.evidence,
        }
    }
}

fn fixtures<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<State>, D::Error> {
    Vec::<Fixture>::deserialize(deserializer)
        .map(|fixtures| fixtures.into_iter().map(State::from).collect())
}

/// One runtime screen: its head, whose ID prefixes every state on it, and the runtime screen it names.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Screen {
    head: Head,
    screen: String,
}

/// The one-line head every entry opens with: its stable ID and the intent an edit must preserve.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Head {
    id: String,
    intent: String,
}

/// A shared rule: its head, where it applies, the expectations it imposes, and what it rests on.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    head: Head,
    scope: Scope,
    expectations: Vec<String>,
    grounds: Grounds,
}

/// Where a rule applies: every state, whole screens by their ID, or named states.
#[derive(Deserialize)]
enum Scope {
    Everywhere,
    Screens(Vec<String>),
    States(Vec<String>),
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
    Intent(String),
    Layout(String),
    Scope(String),
    Misplaced(String),
    Fixture(String),
}

const PLACEHOLDERS: [&str; 6] = ["todo", "tbd", "fixme", "xxx", "placeholder", "lorem"];

const RETELLINGS: [&str; 8] = [
    "show", "shows", "display", "displays", "render", "renders", "list", "lists",
];

impl Catalog {
    /// Decode the authored RON document without accepting unknown fields.
    pub(super) fn try_from_document(document: &str) -> Result<Self> {
        ron::from_str(document).context("Cannot decode TUI intent catalog")
    }

    /// Check that every prepared gallery scene is an authored state or fixture.
    #[must_use]
    pub(super) fn inspect_scenes(&self, names: &[String]) -> Vec<Violation> {
        let scenes = self.scenes();
        names
            .iter()
            .filter(|name| !scenes.iter().any(|scene| &scene.head.id == *name))
            .map(|name| Violation::UnknownState(name.clone()))
            .collect()
    }

    /// Compare each prepared scene's rendered header and footer with its catalog state.
    ///
    /// `renders` pairs a fixture ID with the plain text the real renderer drew for it.
    #[must_use]
    pub(super) fn inspect_renders(&self, renders: &[(String, String)]) -> Vec<Violation> {
        let mut faults = Vec::new();
        let scenes = self.scenes();
        for (id, screen) in renders {
            let Some(state) = scenes.iter().find(|scene| &scene.head.id == id) else {
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
        let scenes = self.scenes();
        identifiers(&self.rules, |rule| &rule.head.id, "rules", &mut faults);
        identifiers(
            &self.map.screens,
            |screen| &screen.head.id,
            "screens",
            &mut faults,
        );
        identifiers(states, |state| &state.head.id, "states", &mut faults);
        if !self.map.fixtures.is_empty() {
            identifiers(
                &self.map.fixtures,
                |fixture| &fixture.head.id,
                "fixtures",
                &mut faults,
            );
        }
        identifiers(
            &self.map.scenarios,
            |scenario| &scenario.head.id,
            "scenarios",
            &mut faults,
        );
        heads(&self.heads(), &mut faults);
        let learner = states
            .iter()
            .map(|state| state.head.id.as_str())
            .collect::<HashSet<_>>();
        let ids = scenes
            .iter()
            .map(|scene| scene.head.id.as_str())
            .collect::<HashSet<_>>();
        let places = self
            .map
            .screens
            .iter()
            .map(|screen| screen.head.id.as_str())
            .collect::<HashSet<_>>();
        for rule in &self.rules {
            expectations(&rule.expectations, &rule.head.id, &mut faults);
            grounds(&rule.grounds, root, &rule.head.id, &mut faults);
            scope(&rule.scope, &places, &learner, &rule.head.id, &mut faults);
        }
        screens_of(&self.map.screens, &scenes, &mut faults);
        prepared(states, &self.map.fixtures, &mut faults);
        for state in &scenes {
            let owner = &state.head.id;
            let contract = &state.contract;
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
                let linked = scenes.iter().any(|state| {
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
        inventory_captures(&scenes, root, &mut faults);
        provenance(&scenes, root, &mut faults);
        faults
    }

    /// Check that every entry's head sits on a single line of `document`.
    ///
    /// That line is what `grep -n 'State(head'` prints, so kind, ID and intent always arrive together.
    #[must_use]
    pub(super) fn inspect_layout(&self, document: &str) -> Vec<Violation> {
        let lines = document.lines().map(str::trim).collect::<Vec<_>>();
        self.heads()
            .into_iter()
            .filter(|(kind, head)| {
                let opening = format!("{kind}(head: Head(id: \"{}\", intent: \"", head.id);
                lines
                    .iter()
                    .filter(|line| line.starts_with(&opening) && line.ends_with("\"),"))
                    .count()
                    != 1
            })
            .map(|(kind, head)| Violation::Layout(format!("{kind} {}", head.id)))
            .collect()
    }

    fn heads(&self) -> Vec<(&'static str, &Head)> {
        self.rules
            .iter()
            .map(|rule| ("Rule", &rule.head))
            .chain(
                self.map
                    .screens
                    .iter()
                    .map(|screen| ("Screen", &screen.head)),
            )
            .chain(self.map.states.iter().map(|state| ("State", &state.head)))
            .chain(
                self.map
                    .fixtures
                    .iter()
                    .map(|fixture| ("Fixture", &fixture.head)),
            )
            .chain(
                self.map
                    .scenarios
                    .iter()
                    .map(|scenario| ("Scenario", &scenario.head)),
            )
            .collect()
    }

    fn scenes(&self) -> Vec<&State> {
        self.map
            .states
            .iter()
            .chain(self.map.fixtures.iter())
            .collect()
    }
}

fn prepared(states: &[State], fixtures: &[State], faults: &mut Vec<Violation>) {
    for state in states {
        for action in &state.contract.actions {
            if fixtures
                .iter()
                .any(|fixture| fixture.head.id == action.target)
            {
                faults.push(Violation::Fixture(format!(
                    "{} -> {} leads a learner into a prepared fixture",
                    state.head.id, action.target
                )));
            }
        }
    }
}

fn heads(values: &[(&'static str, &Head)], faults: &mut Vec<Violation>) {
    let mut kinds = HashMap::new();
    let mut intents = HashMap::new();
    for (kind, head) in values {
        let owner = format!("{kind} {}", head.id);
        if let Some(other) = kinds.insert(head.id.as_str(), *kind)
            && other != *kind
        {
            faults.push(Violation::Duplicate(format!(
                "{} names both a {other} and a {kind}",
                head.id
            )));
        }
        text(&head.intent, &owner, faults);
        intent(&head.intent, &owner, faults);
        if let Some(other) = intents.insert(head.intent.as_str(), owner.clone()) {
            faults.push(Violation::Duplicate(format!(
                "{owner} repeats the intent of {other}"
            )));
        }
    }
}

fn intent(value: &str, owner: &str, faults: &mut Vec<Violation>) {
    if value.trim().is_empty() {
        return;
    }
    let lowered = value.to_lowercase();
    let words = lowered.split_whitespace().collect::<Vec<_>>();
    let placeholder = lowered.contains("...")
        || lowered.contains('…')
        || lowered
            .split(|symbol: char| !symbol.is_ascii_alphanumeric())
            .any(|token| PLACEHOLDERS.contains(&token));
    let problem = if placeholder {
        Some("is a placeholder")
    } else if words.len() < 8 {
        Some("names an action instead of the value it protects")
    } else if value.chars().count() > 180 {
        Some("is too long to read on one grep line")
    } else if words.first().is_some_and(|word| RETELLINGS.contains(word)) {
        Some("retells the screen instead of its value")
    } else {
        None
    };
    if let Some(problem) = problem {
        faults.push(Violation::Intent(format!("{owner} {problem}: {value:?}")));
    }
}

fn scope(
    value: &Scope,
    places: &HashSet<&str>,
    states: &HashSet<&str>,
    owner: &str,
    faults: &mut Vec<Violation>,
) {
    let (names, known, kind) = match value {
        Scope::Everywhere => return,
        Scope::Screens(names) => (names, places, "screen"),
        Scope::States(names) => (names, states, "state"),
    };
    if names.is_empty() {
        faults.push(Violation::Scope(format!("{owner} names no {kind}")));
    }
    for name in names {
        if !known.contains(name.as_str()) {
            faults.push(Violation::Scope(format!(
                "{owner} names unknown {kind} {name}"
            )));
        }
    }
}

fn screens_of(screens: &[Screen], states: &[&State], faults: &mut Vec<Violation>) {
    for screen in screens {
        if !runtime_screens()
            .iter()
            .any(|runtime| screen_name(*runtime) == screen.screen)
        {
            faults.push(Violation::UnknownSurface(format!(
                "screen {} names {}, which is not a runtime screen",
                screen.head.id, screen.screen
            )));
        }
    }
    for runtime in runtime_screens() {
        let name = screen_name(runtime);
        let entries = screens
            .iter()
            .filter(|screen| screen.screen == name)
            .count();
        if entries != 1 {
            faults.push(Violation::MissingSurface(format!(
                "screen {name} has {entries} catalog entries instead of one"
            )));
        }
    }
    for state in states {
        let prefix = state.head.id.split('.').next().unwrap_or_default();
        let placed = screens.iter().any(|screen| {
            screen.head.id == prefix && screen.screen == state.contract.surface.screen
        });
        if !placed {
            faults.push(Violation::Misplaced(format!(
                "{} lives on {} but its prefix names no screen entry for it",
                state.head.id, state.contract.surface.screen
            )));
        }
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
    if !runtime_screens()
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
    for screen in runtime_screens() {
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

fn inventory_captures(states: &[&State], root: &Path, faults: &mut Vec<Violation>) {
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

fn provenance(states: &[&State], root: &Path, faults: &mut Vec<Violation>) {
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

fn runtime_screens() -> [tui::Screen; 5] {
    [
        tui::Screen::Welcome,
        tui::Screen::YourWords,
        tui::Screen::WhatIUnderstood,
        tui::Screen::YourCards,
        tui::Screen::Done,
    ]
}

fn screen_name(screen: tui::Screen) -> &'static str {
    match screen {
        tui::Screen::Welcome => "Welcome",
        tui::Screen::YourWords => "YourWords",
        tui::Screen::WhatIUnderstood => "WhatIUnderstood",
        tui::Screen::YourCards => "YourCards",
        tui::Screen::Done => "Done",
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
        Overlay, Rule, Scenario, Scope, Screen, Shot, Source, State, Surface, Verification,
        Violation,
    };

    fn head(id: &str, intent: &str) -> Head {
        Head {
            id: id.to_owned(),
            intent: intent.to_owned(),
        }
    }

    fn purpose(id: &str) -> String {
        format!("The prepared card {id} stays readable while the learner walks the batch")
    }

    fn state(id: &str) -> State {
        State {
            head: head(id, &purpose(id)),
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
            map: Map {
                screens: vec![Screen {
                    head: head(
                        "cards",
                        "One committed batch is followed from its first card to the saved files",
                    ),
                    screen: String::from("YourCards"),
                }],
                states,
                fixtures: Vec::new(),
                scenarios,
            },
        }
    }

    fn rule(id: &str, scope: Scope) -> Rule {
        Rule {
            head: head(
                id,
                &format!("The shared rule {id} keeps every card readable in the batch"),
            ),
            scope,
            expectations: vec![String::from("Cards stay aligned")],
            grounds: Grounds {
                rationale: Vec::new(),
                sources: vec![Source {
                    file: "render.rs".into(),
                    symbol: String::from("render"),
                }],
                tests: Vec::new(),
                gaps: Vec::new(),
            },
        }
    }

    fn intended(intent: &str) -> Catalog {
        let mut entry = state("cards.open");
        entry.head.intent = intent.to_owned();
        catalog(vec![entry], Vec::new())
    }

    fn rejects_intent(intent: &str) -> bool {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        intended(intent)
            .inspect(directory.path())
            .iter()
            .any(|fault| matches!(fault, Violation::Intent(_)))
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
            head: head(
                "cards.inspect",
                "Reading a card keeps its head focused while the walk moves on",
            ),
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

    #[test]
    fn catalog_cannot_accept_an_empty_intent() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        assert!(
            intended("  ")
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::Empty(_))),
            "catalog accepted an entry whose intent is blank"
        );
    }

    #[test]
    fn catalog_cannot_accept_a_placeholder_intent() {
        assert!(
            rejects_intent("TODO describe what this state protects for the learner later"),
            "catalog accepted a placeholder as an intent"
        );
    }

    #[test]
    fn catalog_cannot_accept_an_intent_that_only_names_an_action() {
        assert!(
            rejects_intent("Wait for credential validation"),
            "catalog accepted an action label as an intent"
        );
    }

    #[test]
    fn catalog_cannot_accept_an_intent_too_long_for_one_grep_line() {
        assert!(
            rejects_intent(&"The learner keeps every typed word intact ".repeat(5)),
            "catalog accepted an intent too long to read on one grep line"
        );
    }

    #[test]
    fn catalog_cannot_accept_an_intent_that_retells_the_screen() {
        assert!(
            rejects_intent("Shows the key field with a submit button and a back hint"),
            "catalog accepted an intent that only retells the screen"
        );
    }

    #[test]
    fn catalog_cannot_accept_two_entries_with_one_intent() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        let mut twin = state("cards.closed");
        twin.head.intent = purpose("cards.open");
        assert!(
            catalog(vec![state("cards.open"), twin], Vec::new())
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::Duplicate(_))),
            "catalog accepted one intent copied onto two entries"
        );
    }

    #[test]
    fn catalog_cannot_reuse_an_id_across_entry_kinds() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        let mut inventory = catalog(vec![state("cards.open")], Vec::new());
        inventory.rules.push(rule("cards.open", Scope::Everywhere));
        assert!(
            inventory
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::Duplicate(_))),
            "catalog let a rule and a state share one ID"
        );
    }

    #[test]
    fn state_cannot_sit_on_a_screen_its_prefix_does_not_name() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        assert!(
            catalog(vec![state("review.open")], Vec::new())
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::Misplaced(_))),
            "a state's ID prefix disagreed with its screen unnoticed"
        );
    }

    #[test]
    fn catalog_cannot_leave_a_runtime_screen_without_an_entry() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        assert!(
            catalog(vec![state("cards.open")], Vec::new())
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::MissingSurface(name) if name.contains("Welcome"))),
            "a runtime screen had no catalog entry and nothing noticed"
        );
    }

    #[test]
    fn rule_scope_cannot_name_an_unknown_state() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        let mut inventory = catalog(vec![state("cards.open")], Vec::new());
        inventory.rules.push(rule(
            "cards.alignment",
            Scope::States(vec![String::from("cards.vanished")]),
        ));
        assert!(
            inventory
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::Scope(_))),
            "a rule scope named a state the catalog does not have"
        );
    }

    #[test]
    fn rule_scope_cannot_be_an_empty_list() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        let mut inventory = catalog(vec![state("cards.open")], Vec::new());
        inventory
            .rules
            .push(rule("cards.alignment", Scope::Screens(Vec::new())));
        assert!(
            inventory
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::Scope(_))),
            "a rule scope listed no screen and still passed"
        );
    }

    #[test]
    fn catalog_cannot_describe_a_screen_the_runtime_does_not_have() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        let mut inventory = catalog(vec![state("cards.open")], Vec::new());
        inventory.map.screens.push(Screen {
            head: head(
                "phantom",
                "A phantom screen keeps every typed word intact while the learner continues",
            ),
            screen: String::from("Phantom"),
        });
        assert!(
            inventory.inspect(directory.path()).iter().any(
                |fault| matches!(fault, Violation::UnknownSurface(name) if name.contains("Phantom"))
            ),
            "catalog described a screen the runtime does not have"
        );
    }

    #[test]
    fn a_learner_action_cannot_lead_into_a_fixture() {
        let directory = tempfile::tempdir().expect("invariant: temporary root must exist");
        let mut entry = state("cards.closed");
        entry.contract.actions.push(Action {
            event: String::from("several attempts fail at once"),
            target: String::from("cards.stress"),
            condition: String::from("always"),
            expectation: String::from("Show the stress layout"),
        });
        let mut inventory = catalog(vec![entry], Vec::new());
        inventory.map.fixtures.push(state("cards.stress"));
        assert!(
            inventory
                .inspect(directory.path())
                .iter()
                .any(|fault| matches!(fault, Violation::Fixture(_))),
            "a learner action led into a prepared fixture unnoticed"
        );
    }

    #[test]
    fn a_prepared_fixture_scene_cannot_be_reported_as_uncatalogued() {
        let mut inventory = catalog(vec![state("cards.open")], Vec::new());
        inventory.map.fixtures.push(state("cards.stress"));
        assert!(
            inventory
                .inspect_scenes(&[String::from("cards.stress")])
                .is_empty(),
            "a gallery scene catalogued as a fixture was reported as unknown"
        );
    }

    #[test]
    fn a_fixture_head_cannot_pass_as_a_state_head() {
        let document = format!(
            "        Screen(head: Head(id: \"cards\", intent: \"One committed batch is followed from its first card to the saved files\"),\n        State(head: Head(id: \"cards.stress\", intent: \"{}\"),\n",
            purpose("cards.stress")
        );
        let mut inventory = catalog(Vec::new(), Vec::new());
        inventory.map.fixtures.push(state("cards.stress"));
        assert!(
            inventory
                .inspect_layout(&document)
                .iter()
                .any(|fault| matches!(fault, Violation::Layout(_))),
            "a fixture laid out as a state head escaped the grep layout check"
        );
    }

    #[test]
    fn catalog_cannot_split_an_entry_head_across_lines() {
        let document = format!(
            "        State(\n            head: Head(id: \"cards.open\", intent: \"{}\"),\n",
            purpose("cards.open")
        );
        assert!(
            catalog(vec![state("cards.open")], Vec::new())
                .inspect_layout(&document)
                .iter()
                .any(|fault| matches!(fault, Violation::Layout(_))),
            "an entry head split across lines escaped the grep layout check"
        );
    }

    #[test]
    fn a_one_line_head_cannot_raise_a_layout_fault() {
        let document = format!(
            "        Screen(head: Head(id: \"cards\", intent: \"One committed batch is followed from its first card to the saved files\"),\n        State(head: Head(id: \"cards.open\", intent: \"{}\"),\n",
            purpose("cards.open")
        );
        assert!(
            catalog(vec![state("cards.open")], Vec::new())
                .inspect_layout(&document)
                .is_empty(),
            "a correctly laid out head was reported as split"
        );
    }
}
