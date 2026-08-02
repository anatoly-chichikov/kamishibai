//! Pure state for editing one card's sentence labels and rewrite note.

use crate::session::{SentenceAxis, SentenceLabelSelection};

/// One focusable row inside the inline sentence-label editor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LabelEditorRow {
    /// The sentence-register chips.
    Register,
    /// The communicative-type chips.
    Type,
    /// The surrounding-language CEFR chips.
    Level,
    /// The free-form rewrite note.
    Note,
}

impl LabelEditorRow {
    /// Return the sentence axis represented by this row.
    #[must_use]
    pub fn axis(self) -> Option<SentenceAxis> {
        match self {
            Self::Register => Some(SentenceAxis::Register),
            Self::Type => Some(SentenceAxis::Type),
            Self::Level => Some(SentenceAxis::Level),
            Self::Note => None,
        }
    }

    /// Return the previous row, saturating at register.
    #[must_use]
    pub fn previous(self) -> Self {
        match self {
            Self::Register | Self::Type => Self::Register,
            Self::Level => Self::Type,
            Self::Note => Self::Level,
        }
    }

    /// Return the next row, saturating at note.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Register => Self::Type,
            Self::Type => Self::Level,
            Self::Level | Self::Note => Self::Note,
        }
    }
}

/// One single-line rewrite note with a UTF-8 byte cursor on a character boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteDraft {
    value: String,
    cursor: usize,
}

impl NoteDraft {
    /// Create a note with its cursor after the supplied text.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.len();
        Self { value, cursor }
    }

    /// Return the note text.
    #[must_use]
    pub fn value(&self) -> &str {
        self.value.as_str()
    }

    /// Return the UTF-8 byte offset of the text cursor.
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Return the note text before the cursor.
    #[must_use]
    pub fn before_cursor(&self) -> &str {
        &self.value[..self.cursor]
    }

    /// Return the note with one character inserted at the cursor.
    #[must_use]
    pub fn typed(mut self, symbol: char) -> Self {
        self.value.insert(self.cursor, symbol);
        self.cursor += symbol.len_utf8();
        self
    }

    /// Return the note with the character before the cursor removed.
    #[must_use]
    pub fn rubbed(mut self) -> Self {
        if self.cursor == 0 {
            return self;
        }
        let previous = previous_boundary(self.value.as_str(), self.cursor);
        self.value.replace_range(previous..self.cursor, "");
        self.cursor = previous;
        self
    }

    /// Return the note with its cursor moved one character left.
    #[must_use]
    pub fn cursor_left(mut self) -> Self {
        self.cursor = previous_boundary(self.value.as_str(), self.cursor);
        self
    }

    /// Return the note with its cursor moved one character right.
    #[must_use]
    pub fn cursor_right(mut self) -> Self {
        if self.cursor < self.value.len() {
            let step = self.value[self.cursor..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(0);
            self.cursor += step;
        }
        self
    }
}

impl Default for NoteDraft {
    fn default() -> Self {
        Self::new("")
    }
}

/// Working sentence labels, focused row, and rewrite note for one card.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SentenceLabelsEditor {
    baseline: SentenceLabelSelection,
    selection: SentenceLabelSelection,
    row: LabelEditorRow,
    note: NoteDraft,
}

impl SentenceLabelsEditor {
    /// Create an editor from generated defaults, working labels, initial focus, and note state.
    #[must_use]
    pub fn new(
        baseline: SentenceLabelSelection,
        selection: SentenceLabelSelection,
        row: LabelEditorRow,
        note: NoteDraft,
    ) -> Self {
        Self {
            baseline,
            selection,
            row,
            note,
        }
    }

    /// Return the current working sentence-label selection.
    #[must_use]
    pub fn selection(&self) -> &SentenceLabelSelection {
        &self.selection
    }

    /// Return the currently focused editor row.
    #[must_use]
    pub fn row(&self) -> LabelEditorRow {
        self.row
    }

    /// Return the current free-form rewrite note.
    #[must_use]
    pub fn note(&self) -> &NoteDraft {
        &self.note
    }

    /// Return the editor focused on one explicit row.
    #[must_use]
    pub fn focused(mut self, row: LabelEditorRow) -> Self {
        self.row = row;
        self
    }

    /// Return the editor focused on the previous row.
    #[must_use]
    pub fn row_previous(mut self) -> Self {
        self.row = self.row.previous();
        self
    }

    /// Return the editor focused on the next row.
    #[must_use]
    pub fn row_next(mut self) -> Self {
        self.row = self.row.next();
        self
    }

    /// Return the editor with the focused axis moved to an adjacent chip.
    #[must_use]
    pub fn axis_advanced(mut self, forward: bool) -> Self {
        if let Some(axis) = self.row.axis() {
            self.selection = self.selection.advanced(axis, forward);
            if self.selection.token(axis) == self.baseline.token(axis) {
                self.selection = self.selection.restoring(axis, &self.baseline);
            }
        }
        self
    }

    /// Return the editor with one chip chosen on the focused axis.
    #[must_use]
    pub fn axis_chosen(mut self, index: usize) -> Self {
        if let Some(axis) = self.row.axis() {
            let active = self.selection.token(axis) == self.selection.choice_token(axis, index);
            self.selection = self.selection.choosing(axis, index);
            let legacy_reset = active && self.baseline.token(axis).is_none();
            if legacy_reset || self.selection.token(axis) == self.baseline.token(axis) {
                self.selection = self.selection.restoring(axis, &self.baseline);
            }
        }
        self
    }

    /// Return the editor with one character inserted when note owns focus.
    #[must_use]
    pub fn typed(mut self, symbol: char) -> Self {
        if self.row == LabelEditorRow::Note {
            self.note = self.note.typed(symbol);
        }
        self
    }

    /// Return the editor with one character removed when note owns focus.
    #[must_use]
    pub fn rubbed(mut self) -> Self {
        if self.row == LabelEditorRow::Note {
            self.note = self.note.rubbed();
        }
        self
    }

    /// Return the editor with the note cursor moved left when note owns focus.
    #[must_use]
    pub fn cursor_left(mut self) -> Self {
        if self.row == LabelEditorRow::Note {
            self.note = self.note.cursor_left();
        }
        self
    }

    /// Return the editor with the note cursor moved right when note owns focus.
    #[must_use]
    pub fn cursor_right(mut self) -> Self {
        if self.row == LabelEditorRow::Note {
            self.note = self.note.cursor_right();
        }
        self
    }
}

fn previous_boundary(value: &str, cursor: usize) -> usize {
    value[..cursor]
        .char_indices()
        .last()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{LabelEditorRow, NoteDraft, SentenceLabelsEditor};
    use crate::session::{Register, SentenceAxis, SentenceLabelSelection};

    #[test]
    fn note_cursor_edits_inside_multibyte_text() {
        let note = NoteDraft::new("café").cursor_left().rubbed().typed('茶');
        assert_eq!(
            (note.value(), note.cursor(), note.before_cursor()),
            ("ca茶é", 5, "ca茶"),
            "the note cursor split a multibyte character while editing"
        );
    }

    #[test]
    fn axis_advance_selects_and_pins_the_first_legacy_choice() {
        let baseline = SentenceLabelSelection::empty();
        let editor = SentenceLabelsEditor::new(
            baseline.clone(),
            baseline,
            LabelEditorRow::Register,
            NoteDraft::default(),
        )
        .axis_advanced(true);
        assert_eq!(
            (
                editor.selection().register(),
                editor.selection().pinned().contains(SentenceAxis::Register),
            ),
            (Some(Register::Neutral), true),
            "advancing an empty register row failed to select and pin its first chip"
        );
    }

    #[test]
    fn row_navigation_saturates_at_both_ends() {
        let first = LabelEditorRow::Register.previous();
        let kind = LabelEditorRow::Register.next();
        let register = LabelEditorRow::Type.previous();
        let level = LabelEditorRow::Type.next();
        let kind_again = LabelEditorRow::Level.previous();
        let note = LabelEditorRow::Level.next();
        let level_again = LabelEditorRow::Note.previous();
        let last = LabelEditorRow::Note.next();
        assert_eq!(
            (
                first,
                kind,
                register,
                level,
                kind_again,
                note,
                level_again,
                last,
            ),
            (
                LabelEditorRow::Register,
                LabelEditorRow::Type,
                LabelEditorRow::Register,
                LabelEditorRow::Level,
                LabelEditorRow::Type,
                LabelEditorRow::Note,
                LabelEditorRow::Level,
                LabelEditorRow::Note
            ),
            "editor row navigation exposed a removed axis or wrapped past one of its ends"
        );
    }

    #[test]
    fn note_input_is_ignored_until_note_owns_focus() {
        let baseline = SentenceLabelSelection::empty();
        let editor = SentenceLabelsEditor::new(
            baseline.clone(),
            baseline,
            LabelEditorRow::Register,
            NoteDraft::default(),
        )
        .typed('x');
        assert_eq!(
            editor.note().value(),
            "",
            "an axis row accepted text intended only for the note"
        );
    }

    #[test]
    fn returning_to_the_generated_chip_restores_its_original_axis_state() {
        let baseline = SentenceLabelSelection::from_labels(&crate::session::SentenceLabels::new(
            Register::Casual,
            crate::session::SentenceLevel::B1,
            crate::session::SentenceKind::Statement,
            crate::session::AxisSet::default(),
            crate::session::AxisSet::default(),
        ));
        let editor = SentenceLabelsEditor::new(
            baseline.clone(),
            baseline,
            LabelEditorRow::Register,
            NoteDraft::default(),
        )
        .axis_chosen(2)
        .axis_chosen(1);
        assert_eq!(
            (
                editor.selection().register(),
                editor.selection().pinned().contains(SentenceAxis::Register),
            ),
            (Some(Register::Casual), false),
            "returning to the generated chip left an incidental pin behind"
        );
    }

    #[test]
    fn repeating_an_active_legacy_chip_restores_the_empty_axis() {
        let baseline = SentenceLabelSelection::empty();
        let editor = SentenceLabelsEditor::new(
            baseline.clone(),
            baseline,
            LabelEditorRow::Register,
            NoteDraft::default(),
        )
        .axis_chosen(0)
        .axis_chosen(0);
        assert_eq!(
            (
                editor.selection().register(),
                editor.selection().pinned().contains(SentenceAxis::Register),
            ),
            (None, false),
            "repeating an active legacy chip failed to restore its empty baseline"
        );
    }

    #[test]
    fn repeating_a_changed_nonlegacy_chip_keeps_the_pending_choice() {
        let baseline = SentenceLabelSelection::from_labels(&crate::session::SentenceLabels::new(
            Register::Casual,
            crate::session::SentenceLevel::B1,
            crate::session::SentenceKind::Statement,
            crate::session::AxisSet::default(),
            crate::session::AxisSet::default(),
        ));
        let editor = SentenceLabelsEditor::new(
            baseline.clone(),
            baseline,
            LabelEditorRow::Register,
            NoteDraft::default(),
        )
        .axis_chosen(2)
        .axis_chosen(2);
        assert_eq!(
            (
                editor.selection().register(),
                editor.selection().pinned().contains(SentenceAxis::Register),
            ),
            (Some(Register::Formal), true),
            "repeating a changed generated chip unexpectedly restored the baseline"
        );
    }
}
