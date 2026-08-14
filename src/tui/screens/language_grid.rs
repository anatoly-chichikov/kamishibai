//! Where the languages sit on the Welcome language step.
//!
//! One shape, measured against the room the screen has: the picker's row —
//! `CODE  Endonym` — repeated across as many columns as the width affords, so a
//! language looks the same wherever it is offered. A screen too narrow or too
//! short for named cells falls back to the bare code, which is the same grid
//! with a smaller cell rather than a second way of choosing a language.
//!
//! The grid answers the three questions the step asks — how many lines it
//! needs, what one line draws, and which language a terminal cell belongs to —
//! so the renderer, the mouse, and the arrow keys all read one layout instead
//! of each inventing its own.

use ratatui::layout::Rect;
use ratatui::text::Span;

use crate::tui::palette;
use crate::tui::picker::PickerSection;

use super::common::pad_right;

/// The half of the pair the step chooses.
const HALF: PickerSection = PickerSection::Known;
/// Cells left blank between two named cells, which end in whatever length the
/// language's own name happens to have.
const NAMED_GAP: usize = 2;
/// Cells the bare-code fallback spends on one language: the code plus the two
/// blanks that make it read as a chip. Those blanks are its own spacing, so
/// code cells sit flush against each other.
const CODE_CELL: usize = 4;

/// What one cell says about a language.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CellShape {
    /// The picker's row: the code, then the language's own name.
    Named,
    /// The bare code, for a screen with no room for a name.
    Code,
}

impl CellShape {
    /// Return the cells one cell of this shape occupies.
    fn width(self) -> usize {
        match self {
            CellShape::Named => HALF.column_width(),
            CellShape::Code => CODE_CELL,
        }
    }

    /// Return the cells left blank after one cell of this shape.
    fn gap(self) -> usize {
        match self {
            CellShape::Named => NAMED_GAP,
            CellShape::Code => 0,
        }
    }

    /// Render one language in this shape.
    fn text(self, index: usize) -> String {
        match self {
            CellShape::Named => HALF.row_text(index),
            CellShape::Code => format!(" {} ", HALF.label_at(index)),
        }
    }
}

/// The supported languages placed inside the room one screen has for them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LanguageGrid {
    shape: CellShape,
    columns: usize,
    count: usize,
}

impl LanguageGrid {
    /// Measure the grid that fits `width` cells across and `rows` lines down.
    ///
    /// Named cells are tried first and kept whenever their lines fit; the bare
    /// code takes over only when they do not, which is the narrow terminal the
    /// step still has to work on.
    #[must_use]
    pub fn measured(width: u16, rows: u16) -> Self {
        let named = Self::packed(CellShape::Named, width);
        if named.lines() <= usize::from(rows).max(1) {
            return named;
        }
        Self::packed(CellShape::Code, width)
    }

    /// Pack every language into `width` cells at one cell shape.
    fn packed(shape: CellShape, width: u16) -> Self {
        let count = HALF.chips();
        let stride = shape.width() + shape.gap();
        let columns = ((usize::from(width) + shape.gap()) / stride).clamp(1, count);
        Self {
            shape,
            columns,
            count,
        }
    }

    /// Return how many lines the grid needs.
    #[must_use]
    pub fn lines(&self) -> usize {
        self.count.div_ceil(self.columns)
    }

    /// Return the cells of one line, with the picked language inverted.
    #[must_use]
    pub fn line(&self, line: usize, picked: usize) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        for column in 0..self.columns {
            let Some(index) = self.language(line, column) else {
                break;
            };
            if column > 0 && self.shape.gap() > 0 {
                spans.push(Span::styled(" ".repeat(self.shape.gap()), palette::base()));
            }
            let style = if index == picked {
                palette::invert()
            } else {
                palette::dim()
            };
            spans.push(Span::styled(
                pad_right(self.shape.text(index).as_str(), self.shape.width()),
                style,
            ));
        }
        spans
    }

    /// Return the language one terminal cell lands on, where `origin` is the
    /// cell the first language is drawn in. A click in the blank between two
    /// cells lands on neither.
    #[must_use]
    pub fn language_at(&self, origin: Rect, x: u16, y: u16) -> Option<usize> {
        let line = usize::from(y.checked_sub(origin.y)?);
        let across = usize::from(x.checked_sub(origin.x)?);
        let stride = self.shape.width() + self.shape.gap();
        if across % stride >= self.shape.width() {
            return None;
        }
        self.language(line, across / stride)
    }

    /// Return the language `rows` lines away from `picked`, staying inside the
    /// grid: a column has a top and a bottom, unlike the wrapping row.
    #[must_use]
    pub fn stepped(&self, picked: usize, rows: i32) -> usize {
        let columns = i32::try_from(self.columns).unwrap_or(1);
        let target = i32::try_from(picked).unwrap_or(0) + rows * columns;
        usize::try_from(target)
            .ok()
            .filter(|moved| *moved < self.count)
            .unwrap_or(picked)
    }

    /// Return the language at one place in the grid, or `None` past its end.
    fn language(&self, line: usize, column: usize) -> Option<usize> {
        if column >= self.columns || line >= self.lines() {
            return None;
        }
        Some(line * self.columns + column).filter(|index| *index < self.count)
    }
}

#[cfg(test)]
mod tests {
    use super::{CellShape, HALF, LanguageGrid};
    use ratatui::layout::Rect;

    /// A wide, tall screen shows the languages by name.
    fn roomy() -> LanguageGrid {
        LanguageGrid::measured(120, 20)
    }

    #[test]
    fn a_roomy_screen_names_every_language() {
        assert_eq!(
            roomy().shape,
            CellShape::Named,
            "a screen with room to spare still hid the language names"
        );
    }

    #[test]
    fn a_short_screen_falls_back_to_the_bare_code() {
        assert_eq!(
            LanguageGrid::measured(40, 3).shape,
            CellShape::Code,
            "a screen too short for named rows must fall back to the codes"
        );
    }

    #[test]
    fn every_language_keeps_a_place_in_the_grid() {
        let grid = roomy();
        let placed = (0..grid.lines())
            .flat_map(|line| {
                (0..grid.columns).filter_map(move |column| grid.language(line, column))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            placed,
            (0..HALF.chips()).collect::<Vec<_>>(),
            "the grid lost a language between its lines"
        );
    }

    #[test]
    fn a_wider_screen_packs_more_languages_side_by_side() {
        assert!(
            LanguageGrid::measured(200, 20).lines() < LanguageGrid::measured(60, 20).lines(),
            "the grid ignored the width it was given"
        );
    }

    #[test]
    fn a_hair_thin_screen_still_offers_one_language_per_line() {
        assert_eq!(
            LanguageGrid::measured(1, 40).lines(),
            HALF.chips(),
            "a screen with room for nothing must still offer one language per line"
        );
    }

    #[test]
    fn clicking_a_cell_lands_on_the_language_drawn_there() {
        let grid = roomy();
        let origin = Rect::new(10, 5, 100, 8);
        let stride = u16::try_from(grid.shape.width() + grid.shape.gap()).expect("stride fits");
        assert_eq!(
            grid.language_at(origin, origin.x + stride, origin.y + 1),
            Some(grid.columns + 1),
            "a click landed on a language the grid does not draw there"
        );
    }

    #[test]
    fn clicking_the_blank_between_two_cells_picks_nothing() {
        let grid = roomy();
        let origin = Rect::new(0, 0, 120, 8);
        let gap = u16::try_from(grid.shape.width()).expect("cell fits");
        assert_eq!(
            grid.language_at(origin, gap, 0),
            None,
            "the blank between two cells answered for a language"
        );
    }

    #[test]
    fn stepping_down_moves_one_whole_line() {
        let grid = roomy();
        assert_eq!(
            grid.stepped(0, 1),
            grid.columns,
            "stepping down must land on the language directly below"
        );
    }

    #[test]
    fn stepping_off_the_top_stays_put() {
        assert_eq!(
            roomy().stepped(1, -1),
            1,
            "stepping above the first line must leave the pick alone"
        );
    }

    #[test]
    fn stepping_off_the_bottom_stays_put() {
        let grid = roomy();
        let last = HALF.chips() - 1;
        assert_eq!(
            grid.stepped(last, 1),
            last,
            "stepping below the last line must leave the pick alone"
        );
    }
}
