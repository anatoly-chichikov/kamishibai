//! Checked one-pass interpolation for embedded prompt templates.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use regex::Regex;

/// One embedded prompt template with named lowercase slots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PromptTemplate<'a> {
    source: &'a str,
}

impl<'a> PromptTemplate<'a> {
    /// Bind one source template for checked interpolation.
    pub(crate) fn new(source: &'a str) -> Self {
        Self { source }
    }

    /// Interpolate every declared slot once without rescanning inserted values.
    pub(crate) fn render(&self, values: &[(&str, String)]) -> Result<String> {
        let source = self.source.trim();
        let pattern = Regex::new(r"\{[a-z_]+\}")?;
        let mut slots = BTreeMap::new();
        for (slot, value) in values {
            if slots.insert(*slot, value.as_str()).is_some() {
                bail!("prompt template received duplicate slot {slot}");
            }
        }
        let mut output = String::with_capacity(source.len());
        let mut seen = BTreeSet::new();
        let mut cursor = 0;
        for found in pattern.find_iter(source) {
            let slot = found.as_str();
            let value = slots
                .get(slot)
                .ok_or_else(|| anyhow::anyhow!("prompt template contains unknown slot {slot}"))?;
            output.push_str(&source[cursor..found.start()]);
            output.push_str(value);
            seen.insert(slot);
            cursor = found.end();
        }
        output.push_str(&source[cursor..]);
        if let Some(slot) = slots.keys().copied().find(|slot| !seen.contains(slot)) {
            bail!("prompt template does not contain supplied slot {slot}");
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::PromptTemplate;

    #[test]
    fn inserted_values_cannot_become_template_slots() {
        let prompt = PromptTemplate::new("{term} / {meaning}")
            .render(&[
                ("{term}", String::from("{meaning}")),
                ("{meaning}", String::from("chosen sense")),
            ])
            .expect("prompt must interpolate");
        assert_eq!(
            prompt, "{meaning} / chosen sense",
            "inserted data was interpreted as template syntax"
        );
    }

    #[test]
    fn unknown_template_slots_fail_fast() {
        let result =
            PromptTemplate::new("{known} {unknown}").render(&[("{known}", String::from("value"))]);
        assert!(
            result.is_err(),
            "an unknown prompt slot escaped template validation"
        );
    }

    #[test]
    fn unused_supplied_slots_fail_fast() {
        let result = PromptTemplate::new("{known}").render(&[
            ("{known}", String::from("value")),
            ("{unused}", String::from("extra")),
        ]);
        assert!(
            result.is_err(),
            "an unused supplied prompt slot escaped template validation"
        );
    }
}
