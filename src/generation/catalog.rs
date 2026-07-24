use anyhow::Result;

use crate::generation::SceneSource;
use crate::generation::manga::Translator;

/// Wrap one scene client with one fixed prompt language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SceneComposer<C> {
    client: C,
    language: String,
    term: String,
    attempt: u8,
}

impl<C> SceneComposer<C> {
    /// Create one scene translator.
    pub fn new(
        client: C,
        language: impl Into<String>,
        term: impl Into<String>,
        attempt: u8,
    ) -> Self {
        Self {
            client,
            language: language.into(),
            term: term.into(),
            attempt,
        }
    }
}

impl<C> Translator for SceneComposer<C>
where
    C: SceneSource,
{
    /// Return one translated scene JSON document.
    fn translate(&self, sentence: &str, target: &str) -> Result<serde_json::Value> {
        self.client.scene(
            self.language.as_str(),
            self.term.as_str(),
            sentence,
            target,
            self.attempt,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use serde_json::{Value, json};

    use super::*;

    #[derive(Clone)]
    struct RecordingSource {
        calls: Rc<RefCell<Vec<String>>>,
    }

    impl SceneSource for RecordingSource {
        fn scene(
            &self,
            language: &str,
            term: &str,
            sentence: &str,
            target: &str,
            attempt: u8,
        ) -> Result<Value> {
            self.calls
                .borrow_mut()
                .push(format!("{language}|{term}|{sentence}|{target}|{attempt}"));
            Ok(json!({}))
        }
    }

    #[test]
    fn scene_composer_preserves_the_term_as_the_layout_seed() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let source = RecordingSource {
            calls: calls.clone(),
        };
        let composer = SceneComposer::new(source, "English", "outlier", 2);
        let _ = composer
            .translate("This point is an outlier", "en")
            .expect("recording source must accept one scene");
        assert_eq!(
            calls.borrow().as_slice(),
            ["English|outlier|This point is an outlier|en|2"],
            "scene composer lost the target term or attempt before layout selection"
        );
    }
}
