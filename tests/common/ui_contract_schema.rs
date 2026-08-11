//! Typed model behind `docs/tui-states/ui-contract.ron`.
//!
//! Loaded by `tests/ui_contract.rs` via `#[path]` for invariant validation.

#![allow(dead_code)]

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Root document.
#[derive(Debug, Deserialize, Serialize)]
pub struct Contract {
    pub meta: Meta,
    pub app: App,
}

/// Provenance and locked-in assumptions for the analysis.
#[derive(Debug, Deserialize, Serialize)]
pub struct Meta {
    pub framework: String,
    pub terminal_size_assumed: TerminalSize,
    pub sources_checked: Vec<PathBuf>,
}

/// Terminal size the snapshots were taken at.
#[derive(Debug, Deserialize, Serialize)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

/// The Kamishibai TUI as a tree of locations.
#[derive(Debug, Deserialize, Serialize)]
pub struct App {
    pub chrome: Chrome,
    pub screens: Vec<Screen>,
    pub modals: Vec<Modal>,
}

/// Elements drawn on every fullscreen screen (badge, header, divider, etc.).
#[derive(Debug, Deserialize, Serialize)]
pub struct Chrome {
    pub elements: Vec<Element>,
}

/// One of the four locked-in fullscreen screens.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, Eq, PartialEq, Hash)]
pub enum ScreenId {
    YourWords,
    WhatIUnderstood,
    YourCards,
    Done,
}

/// The bulk-correction modal represented in the UI inventory.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, Eq, PartialEq, Hash)]
pub enum ModalId {
    ChangeSomething,
}

/// A fullscreen screen, with the states it can render and the regions it paints.
#[derive(Debug, Deserialize, Serialize)]
pub struct Screen {
    pub id: ScreenId,
    pub purpose: String,
    pub states: Vec<ScreenState>,
    pub regions: Vec<Region>,
}

/// Lifecycle states a screen can render in.
#[derive(Debug, Deserialize, Serialize)]
pub enum ScreenState {
    Default,
    Pending,
    ConfirmingClear,
    ConfirmingStop,
    Stopping,
    Partial,
    Empty(EmptyCause),
    Retrying,
    Failed,
    EditingSentenceSettings,
    EditingLabels,
    Regenerating,
}

/// Why an Empty state is being shown.
#[derive(Debug, Deserialize, Serialize)]
pub enum EmptyCause {
    AllDropped,
    NoCards,
    AwaitingDetection,
}

/// A modal overlay rendered above a base screen.
#[derive(Debug, Deserialize, Serialize)]
pub struct Modal {
    pub id: ModalId,
    pub over: ScreenId,
    pub purpose: String,
    pub regions: Vec<Region>,
}

/// A horizontal slice of a screen (header / body / footer / banner / modal).
#[derive(Debug, Deserialize, Serialize)]
pub struct Region {
    pub kind: RegionKind,
    pub elements: Vec<Element>,
}

/// Where a region sits on the screen.
#[derive(Debug, Deserialize, Serialize)]
pub enum RegionKind {
    Chrome,
    Header,
    Body,
    Status,
    Footer,
    Banner,
    Modal,
}

/// One visible UI atom — text, key hint, banner, etc.
#[derive(Debug, Deserialize, Serialize)]
pub struct Element {
    pub id: ElementId,
    pub kind: ElementKind,
    pub text: TextSpec,
    pub source: SourceRef,
    pub purpose: String,
    pub importance: Score,
    pub usefulness: Score,
    pub health: Health,
    pub issues: Vec<Issue>,
}

/// Stable handle for an `Element` (snake.dot.case).
#[derive(Debug, Deserialize, Serialize, Clone, Eq, PartialEq, Hash)]
pub struct ElementId(pub String);

/// A weight in the inclusive 1..=5 range — validated at deserialize, serialized as a bare integer.
#[derive(Debug, Clone, Copy)]
pub struct Score(pub u8);

impl<'de> Deserialize<'de> for Score {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let n = u8::deserialize(d)?;
        if (1..=5).contains(&n) {
            Ok(Score(n))
        } else {
            Err(serde::de::Error::custom(format!(
                "Score must be in 1..=5, got {n}"
            )))
        }
    }
}

impl Serialize for Score {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(s)
    }
}

/// What kind of UI atom an `Element` is.
#[derive(Debug, Deserialize, Serialize)]
pub enum ElementKind {
    Badge,
    HeaderTitle,
    HeaderTagline,
    DashedDivider,
    SectionHeader,
    Rule(RuleStyle),
    Hint,
    Prompt,
    TextArea,
    ListRow,
    DetailBlock,
    StatusLine,
    KeyHint(HintPlacement),
    HiddenKeyBinding,
    BannerFrame,
    BannerTitle,
    BannerSubtitle,
    ModalFrame,
    ModalLabel,
    ModalCardPreview,
    ModalFooter,
}

/// Visual style of a `Rule`.
#[derive(Debug, Deserialize, Serialize)]
pub enum RuleStyle {
    Solid,
    Dashed,
    DoubleEdge,
}

/// Where a `KeyHint` is rendered.
#[derive(Debug, Deserialize, Serialize)]
pub enum HintPlacement {
    Footer,
    Banner,
    BodyInline,
}

/// The text content of an element — a literal or a template with bindings.
#[derive(Debug, Deserialize, Serialize)]
pub enum TextSpec {
    Literal(String),
    Template(Vec<TextFragment>),
}

/// One piece of a template — static text or a typed binding to app data.
#[derive(Debug, Deserialize, Serialize)]
pub enum TextFragment {
    Static(String),
    Bind(DataPath),
}

/// A symbolic data source like `App::pair().learning()`.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DataPath(pub String);

/// A pointer back into the source tree.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SourceRef {
    pub file: PathBuf,
    pub line: u32,
}

/// Single-valued health verdict for an element.
#[derive(Debug, Deserialize, Serialize)]
pub enum Health {
    Working,
    Partial(PartialDegree),
    Broken,
    Fake,
    Decorative,
}

/// What kind of partial-working we're dealing with.
#[derive(Debug, Deserialize, Serialize)]
pub enum PartialDegree {
    CoversSomeStates,
    MisleadingCopy,
    MissingHint,
    ApproximationOfDesign,
}

/// One typed problem attached to an element. The variant *is* the
/// classification — there is no free-form fallback.
#[derive(Debug, Deserialize, Serialize)]
pub enum Issue {
    BrokenWiring {
        handler_at: Option<SourceRef>,
        missing_input: MissingInput,
    },
    HiddenBinding {
        key: String,
        handler_at: SourceRef,
        screen: ScreenId,
    },
    FakeData {
        literal: String,
        should_derive_from: Option<DataPath>,
    },
    DuplicateOf {
        other: ElementId,
        reason: DuplicateReason,
    },
    DriftFromDesign {
        design_ref: PathBuf,
        what_differs: DriftKind,
    },
    SpecPromiseUnfulfilled {
        spec_ref: SourceRef,
        what: String,
    },
    CopyInconsistency {
        with: Vec<ElementId>,
        variants: Vec<String>,
    },
    MisleadingLabel {
        label: String,
        actual_source: DataPath,
    },
}

/// Why a `BrokenWiring` exists — what input path is missing.
/// The `No` prefix names *something that does not exist in the source*;
/// we keep it and silence the lint.
#[derive(Debug, Deserialize, Serialize)]
#[allow(clippy::enum_variant_names)]
pub enum MissingInput {
    NoKeyMapInPromote { key: String, screen: ScreenId },
    NoTransitionArm { event: String, screen: ScreenId },
    NoUiPathForEvent { event: String },
}

/// In what sense two elements duplicate each other.
#[derive(Debug, Deserialize, Serialize)]
pub enum DuplicateReason {
    SameKeyHint,
    SameCopy,
    NearIdenticalCopy,
    SubsetOfOther,
}

/// How the live render diverges from the locked-in PDF design.
#[derive(Debug, Deserialize, Serialize)]
pub enum DriftKind {
    MissingVisualEmphasis { what: String },
    LayoutCollapsed { expected: String, actual: String },
    DecorativeWherePdfCarriesData { suggestion: String },
}
