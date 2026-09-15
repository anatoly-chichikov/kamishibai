//! Compose generation adapters with explicit dependencies.

use std::path::PathBuf;
use std::sync::Arc;

use crate::application::{CardUseCases, CardWorkflow, GenerationCostLedger};
use crate::generation::GeminiCardProduction;
use crate::generation::manga::NativeOutput;
use crate::languages::catalog;
use crate::publishing::{StudyPackagePublisher, SystemPublicationClock};

use super::{GeminiAccess, GeminiClient, GeminiUnderstanding, HttpTransport};

/// Compose the built-in workflow using an explicit client and job-owned paths.
///
/// All operations block the calling thread. Run them in a background worker.
/// The caller supplies a cache root and output directory isolated per tenant
/// and mutable job; profiles are additionally separated beneath that root.
/// Resume a job with the same paths and profile revision. Changing a prompt
/// policy requires changing its revision. No preferences or environment are read
/// here; construct the client with an explicit profile to avoid its legacy
/// environment-aware constructor. Pass a ledger to persist each request's usage.
#[must_use]
pub fn workflow(
    client: GeminiClient<HttpTransport>,
    cache: PathBuf,
    output: PathBuf,
    ledger: Option<Arc<dyn GenerationCostLedger>>,
) -> impl CardUseCases + Clone {
    let cache = cache.join("profiles").join(client.profile_identity());
    let access = GeminiAccess::from_client(client);
    CardWorkflow::new(
        GeminiUnderstanding::new(access.clone(), cache.clone(), ledger.clone()),
        GeminiCardProduction::from_output(
            cache.clone(),
            catalog(),
            access,
            ledger,
            NativeOutput::Preserve,
        ),
        StudyPackagePublisher::new(cache, output, SystemPublicationClock),
    )
}
