//! Composition root for the concrete Gemini-backed application workflow.

use std::path::PathBuf;
use std::sync::Arc;

use crate::application::{CardWorkflow, GenerationCostLedger};
use crate::gemini::{GeminiAccess, GeminiUnderstanding};
use crate::generation::GeminiCardProduction;
use crate::languages::catalog;
use crate::publishing::{StudyPackagePublisher, SystemPublicationClock};

use super::session::SessionCostScope;

/// Concrete workflow used by the interactive and console delivery surfaces.
pub(super) type GeminiCardWorkflow =
    CardWorkflow<GeminiUnderstanding, GeminiCardProduction, StudyPackagePublisher>;

/// Concrete key-validation capability hidden behind the composition root.
pub(super) type GeminiKeyValidation = GeminiAccess;

/// Interactive dependencies with separate card and credential capabilities.
pub(super) struct InteractiveApplication {
    workflow: GeminiCardWorkflow,
    keys: GeminiAccess,
}

impl InteractiveApplication {
    /// Compose one interactive dependency set.
    #[must_use]
    fn new(workflow: GeminiCardWorkflow, keys: GeminiAccess) -> Self {
        Self { workflow, keys }
    }

    /// Consume the dependency set into its independent capabilities.
    #[must_use]
    pub(super) fn into_parts(self) -> (GeminiCardWorkflow, GeminiAccess) {
        (self.workflow, self.keys)
    }
}

/// Compose the interactive card workflow and saved-key validation.
pub(super) fn interactive_application(
    cache: PathBuf,
    output: PathBuf,
    costs: SessionCostScope,
) -> InteractiveApplication {
    let keys = GeminiAccess::interactive();
    let workflow = compose(cache, output, keys, Some(cost_ledger(costs)));
    InteractiveApplication::new(workflow, keys)
}

/// Compose the console workflow with environment-first key access.
pub(super) fn console_workflow(cache: PathBuf, output: PathBuf) -> GeminiCardWorkflow {
    compose(cache, output, GeminiAccess::console(), None)
}

/// Compose a console session whose spend is attributed to stable card slots.
pub(super) fn session_workflow(
    cache: PathBuf,
    output: PathBuf,
    costs: SessionCostScope,
) -> GeminiCardWorkflow {
    compose(
        cache,
        output,
        GeminiAccess::console(),
        Some(cost_ledger(costs)),
    )
}

fn compose(
    cache: PathBuf,
    output: PathBuf,
    access: GeminiAccess,
    costs: Option<Arc<dyn GenerationCostLedger>>,
) -> GeminiCardWorkflow {
    CardWorkflow::new(
        GeminiUnderstanding::new(access, cache.clone()),
        GeminiCardProduction::from_gemini(cache.clone(), catalog(), access, costs),
        StudyPackagePublisher::new(cache, output, SystemPublicationClock),
    )
}

fn cost_ledger(costs: SessionCostScope) -> Arc<dyn GenerationCostLedger> {
    Arc::new(costs)
}
