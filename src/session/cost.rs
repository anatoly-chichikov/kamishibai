//! Dollar-denominated request cost records for generated card artifacts.

use std::iter::Sum;
use std::ops::Add;

use serde::{Deserialize, Serialize};

const NANOS_PER_DISPLAY_UNIT: u64 = 100_000;
const NANOS_PER_CENT: u64 = 10_000_000;

/// Estimated Gemini request cost in nanodollars.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub struct GenerationCost {
    nanos: u64,
    #[serde(default, skip_serializing_if = "is_false")]
    incomplete: bool,
}

impl GenerationCost {
    /// Create a cost from nanodollars.
    #[must_use]
    pub fn from_nanos(nanos: u64) -> Self {
        Self {
            nanos,
            incomplete: false,
        }
    }

    /// Return a zero-cost value.
    #[must_use]
    pub fn zero() -> Self {
        Self::from_nanos(0)
    }

    /// Record provider work whose price cannot be estimated.
    #[must_use]
    pub fn unknown() -> Self {
        Self {
            nanos: 0,
            incomplete: true,
        }
    }

    /// Return the known nanodollar subtotal; use `total` for a complete estimate.
    #[must_use]
    pub fn nanos(&self) -> u64 {
        self.nanos
    }

    /// Return a complete nanodollar estimate only when every request was priced.
    #[must_use]
    pub fn total(&self) -> Option<u64> {
        (!self.incomplete).then_some(self.nanos)
    }

    /// Return a compact USD label for terminal display.
    #[must_use]
    pub fn dollars(&self) -> String {
        if self.incomplete {
            return self.partial(false);
        }
        let rounded =
            self.nanos.saturating_add(NANOS_PER_DISPLAY_UNIT / 2) / NANOS_PER_DISPLAY_UNIT;
        if rounded == 0 {
            return String::from("$0");
        }
        let whole = rounded / 10_000;
        let fraction = rounded % 10_000;
        if whole == 0 {
            return format!("$.{fraction:04}");
        }
        format!("${whole}.{fraction:04}")
    }

    /// Return a USD label rounded to cents for summary totals.
    #[must_use]
    pub fn dollars_cents(&self) -> String {
        if self.incomplete {
            return self.partial(true);
        }
        let rounded = self.nanos.saturating_add(NANOS_PER_CENT / 2) / NANOS_PER_CENT;
        let whole = rounded / 100;
        let cents = rounded % 100;
        format!("${whole}.{cents:02}")
    }

    fn partial(&self, cents: bool) -> String {
        if self.nanos == 0 {
            return String::from("cost unknown");
        }
        let known = Self::from_nanos(self.nanos);
        let label = if cents {
            known.dollars_cents()
        } else {
            known.dollars()
        };
        format!("≥{label}")
    }
}

fn is_false(value: &bool) -> bool {
    !value
}

impl Add for GenerationCost {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            nanos: self.nanos.saturating_add(rhs.nanos),
            incomplete: self.incomplete || rhs.incomplete,
        }
    }
}

impl Sum for GenerationCost {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::zero(), Add::add)
    }
}

/// One stored Gemini usage/cost aggregate for a generated artifact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CostRecord {
    model: String,
    requests: u32,
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    cost: GenerationCost,
    #[serde(default, skip_serializing_if = "is_false")]
    missing_usage: bool,
}

impl CostRecord {
    /// Create one artifact cost record from measured token counts.
    #[must_use]
    pub fn new(
        model: impl Into<String>,
        requests: u32,
        input_tokens: u64,
        output_tokens: u64,
        total_tokens: u64,
        cost: GenerationCost,
    ) -> Self {
        Self {
            model: model.into(),
            requests,
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
            missing_usage: false,
        }
    }

    /// Record one completed provider request that omitted token usage metadata.
    #[must_use]
    pub fn unreported(model: impl Into<String>) -> Self {
        Self {
            missing_usage: true,
            ..Self::new(model, 1, 0, 0, 0, GenerationCost::unknown())
        }
    }

    /// Return the Gemini model id used for this cost.
    #[must_use]
    pub fn model(&self) -> &str {
        self.model.as_str()
    }

    /// Return how many Gemini requests contributed to this aggregate.
    #[must_use]
    pub fn requests(&self) -> u32 {
        self.requests
    }

    /// Return the estimate, whose `total` is absent if any request was unpriced.
    #[must_use]
    pub fn cost(&self) -> GenerationCost {
        self.cost
    }

    /// Return the record merged with another cost record for the same artifact.
    #[must_use]
    pub fn merged(&self, other: &Self) -> Self {
        let model = if self.model == other.model {
            self.model.clone()
        } else {
            format!("{},{}", self.model, other.model)
        };
        Self {
            model,
            requests: self.requests.saturating_add(other.requests),
            input_tokens: self.input_tokens.saturating_add(other.input_tokens),
            output_tokens: self.output_tokens.saturating_add(other.output_tokens),
            total_tokens: self.total_tokens.saturating_add(other.total_tokens),
            cost: self.cost + other.cost,
            missing_usage: self.missing_usage || other.missing_usage,
        }
    }

    /// Return one aggregate record for a non-empty sequence.
    #[must_use]
    pub fn aggregate(records: &[Self]) -> Option<Self> {
        let mut iter = records.iter();
        let first = iter.next()?.clone();
        Some(iter.fold(first, |total, item| total.merged(item)))
    }
}

#[cfg(test)]
mod tests {
    use super::{CostRecord, GenerationCost};

    #[test]
    fn unknown_pricing_cannot_become_a_complete_total_after_aggregation() {
        let known = CostRecord::new("priced", 1, 37, 19, 56, GenerationCost::from_nanos(731_000));
        let unknown = CostRecord::new("unpriced", 1, 83, 11, 94, GenerationCost::unknown());
        let merged = known.merged(&unknown);
        assert_eq!(
            (
                merged.requests(),
                merged.cost().nanos(),
                merged.cost().total()
            ),
            (2, 731_000, None),
            "mixed usage erased requests or exposed the known subtotal as a complete estimate"
        );
    }

    #[test]
    fn unknown_estimates_roundtrip_without_claiming_zero_dollars() {
        let cost = GenerationCost::unknown() + GenerationCost::from_nanos(170_000);
        let value = serde_json::to_value(cost).expect("partial estimate must encode");
        let restored: GenerationCost =
            serde_json::from_value(value.clone()).expect("partial estimate must decode");
        assert_eq!(
            (value, restored.total(), restored.dollars()),
            (
                serde_json::json!({"nanos": 170_000, "incomplete": true}),
                None,
                String::from("≥$.0002")
            ),
            "an unknown estimate lost its completeness state across persistence"
        );
    }

    #[test]
    fn old_cost_documents_remain_complete_estimates() {
        let cost: GenerationCost =
            serde_json::from_str("{\"nanos\":713000}").expect("old cost must decode");
        assert_eq!(
            cost.total(),
            Some(713_000),
            "an existing priced cost became unknown after loading"
        );
    }

    #[test]
    fn dollars_cents_keeps_the_zero_before_subdollar_totals() {
        assert_eq!(
            GenerationCost::from_nanos(80_800_000).dollars_cents(),
            "$0.08",
            "subdollar summary totals must keep the leading zero"
        );
    }

    #[test]
    fn dollars_cents_rounds_to_the_nearest_cent() {
        assert_eq!(
            GenerationCost::from_nanos(1_015_000_000).dollars_cents(),
            "$1.02",
            "summary totals must round to ordinary cent precision"
        );
    }
}
