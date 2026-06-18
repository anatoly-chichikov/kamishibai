//! Dollar-denominated request cost records for generated card artifacts.

use std::iter::Sum;
use std::ops::Add;

use serde::{Deserialize, Serialize};

const NANOS_PER_DISPLAY_UNIT: u64 = 100_000;

/// Estimated Gemini request cost in nanodollars.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct GenerationCost {
    nanos: u64,
}

impl GenerationCost {
    /// Create a cost from nanodollars.
    #[must_use]
    pub fn from_nanos(nanos: u64) -> Self {
        Self { nanos }
    }

    /// Return a zero-cost value.
    #[must_use]
    pub fn zero() -> Self {
        Self { nanos: 0 }
    }

    /// Return the raw nanodollar amount.
    #[must_use]
    pub fn nanos(&self) -> u64 {
        self.nanos
    }

    /// Return a compact USD label for terminal display.
    #[must_use]
    pub fn dollars(&self) -> String {
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
}

impl Add for GenerationCost {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            nanos: self.nanos.saturating_add(rhs.nanos),
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

    /// Return the estimated dollar cost.
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
