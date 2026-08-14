//! Vocabulary for choosing the session language pair.
//!
//! The pair is one concept, so one modal owns both halves. `PickerSection`
//! names a half, `PickerCursor` remembers the highlighted chip of *both*
//! halves plus which half has focus, and `LanguageChoice` is what a confirmed
//! pick hands to the shell.
//!
//! The learning half carries one extra leading chip — `auto` — which is the
//! way back to `LearningTarget::Detect` after a pin.

use std::sync::OnceLock;

use crate::application::LearningTarget;
use crate::languages::catalog;
use crate::tui::screens::common::{display_width, pad_right};

/// The label of the learning half's leading row, which means "detect again".
pub const AUTO_CHIP: &str = "auto";

/// Cells the code cell occupies, sized for the widest label (`auto`).
pub const CODE_WIDTH: usize = 4;

/// What that leading row promises in the name column.
pub const AUTO_NAME: &str = "detect from the words";

/// Which half of the language pair an interaction addresses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickerSection {
    /// The language the learner already knows and reads explanations in.
    Known,
    /// The language the learner is studying in this batch.
    Learning,
}

impl PickerSection {
    /// Return how many chips this half renders.
    #[must_use]
    pub fn chips(self) -> usize {
        match self {
            PickerSection::Known => catalog().codes().len(),
            PickerSection::Learning => catalog().codes().len() + 1,
        }
    }

    /// Return the supported code behind one chip, or `None` for the `auto`
    /// chip that only the learning half carries.
    #[must_use]
    pub fn code_at(self, index: usize) -> Option<&'static str> {
        let codes = catalog().codes();
        match self {
            PickerSection::Known => codes.get(index).copied(),
            PickerSection::Learning => index
                .checked_sub(1)
                .and_then(|shifted| codes.get(shifted).copied()),
        }
    }

    /// Return the chip index that carries one supported code.
    #[must_use]
    pub fn chip_for(self, code: &str) -> usize {
        let found = catalog()
            .codes()
            .iter()
            .position(|item| item.eq_ignore_ascii_case(code));
        match self {
            PickerSection::Known => found.unwrap_or(0),
            PickerSection::Learning => found.map_or(0, |index| index + 1),
        }
    }

    /// Return the first index this half scrolls, leaving anything before it
    /// pinned above the list.
    ///
    /// Only the learning half pins a row — `auto`. Because both halves are left
    /// with exactly the catalog behind that line, the two lists of languages
    /// line up row for row by construction rather than by tuning offsets.
    #[must_use]
    pub fn scrolling_first(self) -> usize {
        match self {
            PickerSection::Known => 0,
            PickerSection::Learning => 1,
        }
    }

    /// Return how many rows this half scrolls.
    #[must_use]
    pub fn scrolling(self) -> usize {
        self.chips() - self.scrolling_first()
    }

    /// Return the code cell of one row: the uppercase code, or `auto`.
    #[must_use]
    pub fn label_at(self, index: usize) -> String {
        self.code_at(index)
            .map_or_else(|| String::from(AUTO_CHIP), str::to_uppercase)
    }

    /// Return the name cell of one row: the language's own name, or what the
    /// `auto` row promises instead.
    ///
    /// Both cells resolve here so the `auto` row is data like every other row
    /// rather than a special case the renderer has to branch on.
    #[must_use]
    pub fn name_at(self, index: usize) -> String {
        let Some(code) = self.code_at(index) else {
            return String::from(AUTO_NAME);
        };
        catalog()
            .borrowed(code)
            .map(|profile| profile.endonym.clone())
            .unwrap_or_else(|_| code.to_uppercase())
    }

    /// Render one row as its two cells: the padded code, then the language's
    /// own name.
    ///
    /// The `auto` row goes through here too, so it lines up with the languages
    /// instead of being a special case. Both the modal's columns and the
    /// Welcome step's grid draw their cells from here, which is what makes one
    /// language look the same wherever it is offered.
    #[must_use]
    pub fn row_text(self, index: usize) -> String {
        format!(
            "{}  {}",
            pad_right(self.label_at(index).as_str(), CODE_WIDTH),
            self.name_at(index)
        )
    }

    /// Return the cells one column of this half occupies.
    ///
    /// A column is as wide as the widest row it can ever hold, which the
    /// catalog decides and no terminal can change — so both halves are measured
    /// once. Every span of every frame and every mouse hit-test asks for these
    /// two numbers, and re-reading two dozen rows per question is what left the
    /// open modal seconds behind the arrow keys.
    #[must_use]
    pub fn column_width(self) -> usize {
        static WIDTHS: OnceLock<[usize; 2]> = OnceLock::new();
        let widths = WIDTHS
            .get_or_init(|| [PickerSection::Known, PickerSection::Learning].map(Self::measured));
        match self {
            PickerSection::Known => widths[0],
            PickerSection::Learning => widths[1],
        }
    }

    /// Measure this half against every row it can hold and its own heading.
    fn measured(self) -> usize {
        (0..self.chips())
            .map(|index| display_width(self.row_text(index).as_str()))
            .max()
            .unwrap_or(0)
            .max(display_width(self.heading()))
    }

    /// Return the heading printed above this half inside the modal.
    #[must_use]
    pub fn heading(self) -> &'static str {
        match self {
            PickerSection::Known => "your language",
            PickerSection::Learning => "what you're learning",
        }
    }

    /// Return the other half.
    #[must_use]
    pub fn other(self) -> Self {
        match self {
            PickerSection::Known => PickerSection::Learning,
            PickerSection::Learning => PickerSection::Known,
        }
    }
}

/// The highlighted chip of both halves plus the half that owns the arrows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PickerCursor {
    known: usize,
    learning: usize,
    section: PickerSection,
}

impl PickerCursor {
    /// Create one cursor from both chip indices and the focused half.
    #[must_use]
    pub fn new(known: usize, learning: usize, section: PickerSection) -> Self {
        Self {
            known,
            learning,
            section,
        }
    }

    /// Create the cursor the modal opens with: the active pair preselected and
    /// the requested half focused. A `None` pin preselects the `auto` chip.
    #[must_use]
    pub fn opening(known: &str, pinned: Option<&str>, section: PickerSection) -> Self {
        let learning = pinned.map_or(0, |code| PickerSection::Learning.chip_for(code));
        Self::new(PickerSection::Known.chip_for(known), learning, section)
    }

    /// Return the focused half.
    #[must_use]
    pub fn section(&self) -> PickerSection {
        self.section
    }

    /// Return the highlighted chip index of one half.
    #[must_use]
    pub fn index(&self, section: PickerSection) -> usize {
        let raw = match section {
            PickerSection::Known => self.known,
            PickerSection::Learning => self.learning,
        };
        raw.min(section.chips().saturating_sub(1))
    }

    /// Return the highlighted chip index of the focused half.
    #[must_use]
    pub fn focused(&self) -> usize {
        self.index(self.section)
    }

    /// Return the cursor advanced by `delta` inside the focused half, wrapping
    /// around that half only.
    #[must_use]
    pub fn advanced(self, delta: i32) -> Self {
        let chips = i32::try_from(self.section.chips()).unwrap_or(1).max(1);
        let next = (i32::try_from(self.focused()).unwrap_or(0) + delta).rem_euclid(chips);
        self.chosen(self.section, usize::try_from(next).unwrap_or(0))
    }

    /// Return the cursor focused on one half, keeping both picks.
    ///
    /// Naming the destination rather than toggling is what makes `←` and `→`
    /// honest: pressing `←` on the left column stays put instead of throwing
    /// focus across to the right.
    #[must_use]
    pub fn facing(self, section: PickerSection) -> Self {
        Self { section, ..self }
    }

    /// Return the cursor with one half focused and its chip highlighted.
    #[must_use]
    pub fn chosen(self, section: PickerSection, index: usize) -> Self {
        let index = index.min(section.chips().saturating_sub(1));
        match section {
            PickerSection::Known => Self {
                known: index,
                section,
                ..self
            },
            PickerSection::Learning => Self {
                learning: index,
                section,
                ..self
            },
        }
    }

    /// Return what confirming this cursor means for the session pair.
    #[must_use]
    pub fn choice(&self) -> LanguageChoice {
        let known = PickerSection::Known
            .code_at(self.index(PickerSection::Known))
            .unwrap_or("en");
        LanguageChoice::new(
            known.to_uppercase(),
            learning_target(PickerSection::Learning.code_at(self.index(PickerSection::Learning))),
        )
    }
}

/// Build the understanding policy for one optionally pinned learning code.
#[must_use]
pub fn learning_target(code: Option<&str>) -> LearningTarget {
    match code {
        None => LearningTarget::Detect,
        Some(code) => LearningTarget::Explicit(
            catalog()
                .resolve(code)
                .expect("invariant: a picked learning code comes from the language catalog"),
        ),
    }
}

/// One confirmed language pair: the known half is always a concrete code, the
/// learning half is either pinned or left to detection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanguageChoice {
    known: String,
    learning: LearningTarget,
}

impl LanguageChoice {
    /// Create one confirmed choice from a known code and a learning policy.
    #[must_use]
    pub fn new(known: impl Into<String>, learning: LearningTarget) -> Self {
        Self {
            known: known.into(),
            learning,
        }
    }

    /// Return the known language code.
    #[must_use]
    pub fn known(&self) -> &str {
        self.known.as_str()
    }

    /// Return the learning policy.
    #[must_use]
    pub fn learning(&self) -> &LearningTarget {
        &self.learning
    }

    /// Return the pinned learning code, or `None` when detection stays in charge.
    #[must_use]
    pub fn pinned(&self) -> Option<&str> {
        match &self.learning {
            LearningTarget::Detect => None,
            LearningTarget::Explicit(code) => Some(code.as_ref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LanguageChoice, PickerCursor, PickerSection, learning_target};
    use crate::application::LearningTarget;
    use crate::languages::catalog;

    #[test]
    fn the_learning_half_carries_one_more_chip_than_the_catalog() {
        assert_eq!(
            PickerSection::Learning.chips() - PickerSection::Known.chips(),
            1,
            "the learning half lost its auto chip"
        );
    }

    /// The two halves scroll the same number of rows. This is what makes the
    /// languages line up across the columns, so it is the thing worth pinning:
    /// the alignment follows from it rather than from the renderer.
    #[test]
    fn both_halves_scroll_the_same_number_of_rows() {
        assert_eq!(
            PickerSection::Known.scrolling(),
            PickerSection::Learning.scrolling(),
            "the halves stopped scrolling the same catalog, so their rows cannot line up"
        );
    }

    #[test]
    fn only_the_learning_half_pins_a_row() {
        assert_eq!(
            (
                PickerSection::Known.scrolling_first(),
                PickerSection::Learning.scrolling_first()
            ),
            (0, 1),
            "the pinned row must belong to the learning half alone"
        );
    }

    #[test]
    fn the_leading_learning_chip_carries_no_code() {
        assert_eq!(
            PickerSection::Learning.code_at(0),
            None,
            "the leading learning chip must mean detection, not a language"
        );
    }

    #[test]
    fn a_learning_chip_resolves_to_the_code_one_position_earlier() {
        assert_eq!(
            PickerSection::Learning.code_at(1),
            PickerSection::Known.code_at(0),
            "the learning half is offset by exactly the auto chip"
        );
    }

    #[test]
    fn opening_without_a_pin_preselects_the_auto_chip() {
        assert_eq!(
            PickerCursor::opening("ru", None, PickerSection::Known).index(PickerSection::Learning),
            0,
            "an unpinned learning half must open on auto"
        );
    }

    #[test]
    fn opening_with_a_pin_preselects_that_language() {
        let cursor = PickerCursor::opening("ru", Some("fr"), PickerSection::Learning);
        assert_eq!(
            PickerSection::Learning.code_at(cursor.index(PickerSection::Learning)),
            Some("fr"),
            "a pinned learning half must open on the pinned language"
        );
    }

    #[test]
    fn advancing_one_half_leaves_the_other_untouched() {
        let cursor = PickerCursor::opening("ru", Some("fr"), PickerSection::Known);
        assert_eq!(
            cursor.advanced(3).index(PickerSection::Learning),
            cursor.index(PickerSection::Learning),
            "moving inside one half disturbed the other half"
        );
    }

    #[test]
    fn advancing_wraps_inside_the_focused_half_only() {
        let cursor = PickerCursor::new(0, 0, PickerSection::Known);
        assert_eq!(
            cursor.advanced(-1).index(PickerSection::Known),
            PickerSection::Known.chips() - 1,
            "the known half must wrap around its own last chip"
        );
    }

    #[test]
    fn facing_the_other_half_keeps_both_picks() {
        let cursor = PickerCursor::new(4, 7, PickerSection::Known).facing(PickerSection::Learning);
        assert_eq!(
            (
                cursor.section(),
                cursor.index(PickerSection::Known),
                cursor.index(PickerSection::Learning)
            ),
            (PickerSection::Learning, 4, 7),
            "changing focus lost a half's pick"
        );
    }

    #[test]
    fn facing_the_half_already_focused_stays_put() {
        let cursor = PickerCursor::new(4, 7, PickerSection::Known).facing(PickerSection::Known);
        assert_eq!(
            cursor.section(),
            PickerSection::Known,
            "focusing the half already focused must not throw focus across"
        );
    }

    #[test]
    fn a_language_row_names_itself_in_its_own_language() {
        assert_eq!(
            PickerSection::Learning.name_at(PickerSection::Learning.chip_for("de")),
            "Deutsch",
            "a language row must name itself the way its own speakers write it"
        );
    }

    #[test]
    fn the_auto_row_promises_detection_instead_of_a_language() {
        assert_eq!(
            PickerSection::Learning.name_at(0),
            super::AUTO_NAME,
            "the auto row must say what it does, not name a language"
        );
    }

    #[test]
    fn confirming_the_auto_chip_means_detection() {
        assert_eq!(
            PickerCursor::opening("ru", None, PickerSection::Known)
                .choice()
                .learning(),
            &LearningTarget::Detect,
            "the auto chip must hand detection back to the pass"
        );
    }

    #[test]
    fn confirming_a_language_chip_pins_that_uppercase_code() {
        assert_eq!(
            PickerCursor::opening("ru", Some("fr"), PickerSection::Known)
                .choice()
                .pinned(),
            Some("FR"),
            "a picked learning chip must pin its canonical code"
        );
    }

    #[test]
    fn confirming_uppercases_the_known_half() {
        assert_eq!(
            PickerCursor::opening("ru", None, PickerSection::Known)
                .choice()
                .known(),
            "RU",
            "the known half must be confirmed in the canonical case"
        );
    }

    #[test]
    fn every_catalog_code_has_a_chip_in_both_halves() {
        let missing = catalog()
            .codes()
            .into_iter()
            .filter(|code| {
                PickerSection::Known.code_at(PickerSection::Known.chip_for(code)) != Some(*code)
                    || PickerSection::Learning.code_at(PickerSection::Learning.chip_for(code))
                        != Some(*code)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            missing,
            Vec::<&'static str>::new(),
            "a supported language lost its chip in one half of the picker"
        );
    }

    #[test]
    fn an_unpinned_choice_reports_no_pinned_code() {
        assert_eq!(
            LanguageChoice::new("EN", learning_target(None)).pinned(),
            None,
            "an unpinned choice must not claim a learning code"
        );
    }
}
