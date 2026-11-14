//! Shared wording an application reuses when it writes its own model tasks in the core's voice.

use anyhow::Result;

use crate::languages::catalog;
use crate::prompt::PromptTemplate;

const LEARNER_EXPLANATIONS: &str = include_str!("../assets/learner_explanations_prompt.txt");

/// Return the rules for learner-facing explanations with examples in the known language.
/// Experimental embedding API; compatibility is not guaranteed.
pub fn explanations(known: &str) -> Result<String> {
    PromptTemplate::new(LEARNER_EXPLANATIONS)
        .render(&[("{learner_examples}", catalog().prompts(known)?.writing()?)])
}

/// Return example scene cues written in the known language.
/// Experimental embedding API; compatibility is not guaranteed.
pub fn hints(known: &str) -> Result<String> {
    catalog().prompts(known)?.hint()
}

/// Return the allowed length of a scene cue in the known language.
/// Experimental embedding API; compatibility is not guaranteed.
pub fn hint_length(known: &str) -> Result<&'static str> {
    Ok(catalog().prompts(known)?.hint_length())
}

/// Return example explanations written in the known language.
/// Experimental embedding API; compatibility is not guaranteed.
pub fn contexts(known: &str) -> Result<String> {
    catalog().prompts(known)?.context()
}

/// Fill every named slot of a template exactly once, refusing unknown and unused slots.
/// Experimental embedding API; compatibility is not guaranteed.
pub fn fill(template: &str, values: &[(&str, String)]) -> Result<String> {
    PromptTemplate::new(template).render(values)
}
