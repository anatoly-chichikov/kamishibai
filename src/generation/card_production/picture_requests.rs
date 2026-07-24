//! Enforces the durable picture-request budget before provider calls.

use std::fs;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::cost_accounting::AccountingHealth;
use crate::generation::artifact_cache::{Cache, PICTURE_REQUESTS_FILE};
use crate::generation::manga::ImageSource;
use crate::session::ARTIFACT_ATTEMPT_CEILING;

/// Return all picture requests made for one visual revision.
pub(super) fn picture_request_total(cache: &Cache) -> Result<u32> {
    load_picture_request_counter(cache).map(|counter| counter.requests)
}

const PICTURE_REQUEST_COUNTER_SCHEMA: &str = "kamishibai.picture-request-counter";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// Tracks lifetime and current-series picture requests for one card.
pub(super) struct PictureRequestCounter {
    schema: String,
    version: u8,
    /// Lifetime requests for this visual revision.
    pub(super) requests: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Requests since the latest explicit regeneration.
    pub(super) series_requests: Option<u32>,
}

impl PictureRequestCounter {
    /// Build a new durable counter value.
    pub(super) fn new(requests: u32, series_requests: u32) -> Self {
        Self {
            schema: String::from(PICTURE_REQUEST_COUNTER_SCHEMA),
            version: 1,
            requests,
            series_requests: Some(series_requests),
        }
    }

    /// Reserve one request before the provider call starts.
    pub(super) fn reserved(&self) -> Result<Self> {
        let series = self.series_requests.unwrap_or(self.requests);
        let ceiling = u32::from(ARTIFACT_ATTEMPT_CEILING);
        if series >= ceiling {
            bail!("picture request series exhausted its {ceiling}-attempt ceiling");
        }
        Ok(Self::new(
            self.requests
                .checked_add(1)
                .ok_or_else(|| anyhow!("picture request counter overflow"))?,
            series
                .checked_add(1)
                .ok_or_else(|| anyhow!("picture request series counter overflow"))?,
        ))
    }

    /// Start a new bounded series while retaining lifetime usage.
    pub(super) fn restarted(&self) -> Self {
        Self::new(self.requests, 0)
    }

    /// Reject unsupported or inconsistent persisted counters.
    pub(super) fn validated(self) -> Result<Self> {
        if self.schema != PICTURE_REQUEST_COUNTER_SCHEMA || self.version != 1 {
            bail!("picture request counter has an unsupported schema");
        }
        if self.series_requests.unwrap_or(self.requests) > self.requests {
            bail!("picture request series exceeds its total request count");
        }
        Ok(self)
    }
}

/// Load and validate the durable request counter.
pub(super) fn load_picture_request_counter(cache: &Cache) -> Result<PictureRequestCounter> {
    if !cache.exists(PICTURE_REQUESTS_FILE) {
        return Ok(PictureRequestCounter::new(0, 0));
    }
    let path = cache.filepath(PICTURE_REQUESTS_FILE)?;
    serde_json::from_slice::<PictureRequestCounter>(fs::read(path)?.as_slice())?.validated()
}

fn store_picture_request_counter(cache: &Cache, counter: &PictureRequestCounter) -> Result<()> {
    let staged = cache.stage(".requests.json")?;
    let result = serde_json::to_vec_pretty(counter)
        .map_err(anyhow::Error::from)
        .and_then(|json| fs::write(&staged, json).map_err(anyhow::Error::from))
        .and_then(|()| cache.commit(&staged, PICTURE_REQUESTS_FILE));
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result
}

/// Durably reserve one request before contacting the image provider.
pub(crate) fn reserve_picture_request(cache: &Cache) -> Result<()> {
    let counter = load_picture_request_counter(cache)?.reserved()?;
    store_picture_request_counter(cache, &counter)
}

/// Reset the bounded series for an explicit picture regeneration.
pub(crate) fn restart_picture_request_series(cache: &Cache) -> Result<()> {
    if cache.exists(PICTURE_REQUESTS_FILE) {
        store_picture_request_counter(cache, &load_picture_request_counter(cache)?.restarted())?;
    }
    Ok(())
}

#[derive(Clone, Debug)]
/// Decorates an image port with durable request reservation.
pub(super) struct RequestCountingImage<C> {
    client: C,
    cache: Cache,
    accounting: AccountingHealth,
}

impl<C> RequestCountingImage<C> {
    #[cfg(test)]
    /// Build a request-counting image port for tests.
    pub(super) fn new(client: C, cache: Cache) -> Self {
        Self::guarded(client, cache, AccountingHealth::default())
    }

    /// Build a request-counting port sharing accounting health.
    pub(super) fn guarded(client: C, cache: Cache, accounting: AccountingHealth) -> Self {
        Self {
            client,
            cache,
            accounting,
        }
    }
}

impl<C> ImageSource for RequestCountingImage<C>
where
    C: ImageSource,
{
    fn image(&self, prompt: &str) -> Result<Vec<u8>> {
        self.accounting
            .record(reserve_picture_request(&self.cache))?;
        self.client.image(prompt)
    }
}
