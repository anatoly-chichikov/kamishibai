//! Paid-tier Gemini request cost estimation from response usage metadata.

use crate::session::{CostRecord, GenerationCost};

use super::protocol::UsageMetadata;

const GEMINI_3_8_FLASH_INPUT_NANOS: u64 = 750;
const GEMINI_3_8_FLASH_OUTPUT_NANOS: u64 = 3_750;
const GEMINI_3_7_FLASH_INPUT_NANOS: u64 = 750;
const GEMINI_3_7_FLASH_OUTPUT_NANOS: u64 = 3_750;
const GEMINI_3_6_FLASH_INPUT_NANOS: u64 = 1_500;
const GEMINI_3_6_FLASH_OUTPUT_NANOS: u64 = 7_500;
const GEMINI_3_5_FLASH_INPUT_NANOS: u64 = 1_500;
const GEMINI_3_5_FLASH_OUTPUT_NANOS: u64 = 9_000;
const GEMINI_3_5_FLASH_LITE_INPUT_NANOS: u64 = 300;
const GEMINI_3_5_FLASH_LITE_OUTPUT_NANOS: u64 = 2_500;
const GEMINI_2_5_FLASH_LITE_INPUT_NANOS: u64 = 100;
const GEMINI_2_5_FLASH_LITE_OUTPUT_NANOS: u64 = 400;
const GEMINI_3_1_FLASH_IMAGE_INPUT_NANOS: u64 = 500;
const GEMINI_3_1_FLASH_IMAGE_OUTPUT_NANOS: u64 = 60_000;
const GEMINI_3_1_FLASH_IMAGE_THINKING_NANOS: u64 = 3_000;
const GEMINI_3_1_FLASH_TTS_INPUT_NANOS: u64 = 1_000;
const GEMINI_3_1_FLASH_TTS_OUTPUT_NANOS: u64 = 20_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Rates {
    input_nanos: u64,
    output_nanos: u64,
    thinking_nanos: u64,
}

impl Rates {
    fn priced(self, usage: &UsageMetadata, model: &str) -> CostRecord {
        let input = usage.prompt_token_count;
        let output = output_tokens(usage);
        let input_cost = input.saturating_mul(self.input_nanos);
        let output_cost = if usage.candidates_token_count > 0 || usage.thoughts_token_count > 0 {
            usage
                .candidates_token_count
                .saturating_mul(self.output_nanos)
                .saturating_add(
                    usage
                        .thoughts_token_count
                        .saturating_mul(self.thinking_nanos),
                )
        } else {
            output.saturating_mul(self.output_nanos)
        };
        CostRecord::new(
            model,
            1,
            input,
            output,
            usage.total_token_count,
            GenerationCost::from_nanos(input_cost.saturating_add(output_cost)),
        )
    }
}

pub(super) fn priced(model: &str, usage: Option<&UsageMetadata>) -> CostRecord {
    match (usage, rates(model)) {
        (Some(usage), Some(rates)) => rates.priced(usage, model),
        (Some(usage), None) => CostRecord::new(
            model,
            1,
            usage.prompt_token_count,
            output_tokens(usage),
            usage.total_token_count,
            GenerationCost::unknown(),
        ),
        (None, _) => CostRecord::unreported(model),
    }
}

fn output_tokens(usage: &UsageMetadata) -> u64 {
    let generated = usage
        .candidates_token_count
        .saturating_add(usage.thoughts_token_count);
    if generated > 0 {
        return generated;
    }
    usage
        .total_token_count
        .saturating_sub(usage.prompt_token_count)
}

fn rates(model: &str) -> Option<Rates> {
    Some(match model {
        "gemini-3.8-flash" => Rates {
            input_nanos: GEMINI_3_8_FLASH_INPUT_NANOS,
            output_nanos: GEMINI_3_8_FLASH_OUTPUT_NANOS,
            thinking_nanos: GEMINI_3_8_FLASH_OUTPUT_NANOS,
        },
        "gemini-3.7-flash" => Rates {
            input_nanos: GEMINI_3_7_FLASH_INPUT_NANOS,
            output_nanos: GEMINI_3_7_FLASH_OUTPUT_NANOS,
            thinking_nanos: GEMINI_3_7_FLASH_OUTPUT_NANOS,
        },
        "gemini-3.6-flash" => Rates {
            input_nanos: GEMINI_3_6_FLASH_INPUT_NANOS,
            output_nanos: GEMINI_3_6_FLASH_OUTPUT_NANOS,
            thinking_nanos: GEMINI_3_6_FLASH_OUTPUT_NANOS,
        },
        "gemini-3.5-flash" => Rates {
            input_nanos: GEMINI_3_5_FLASH_INPUT_NANOS,
            output_nanos: GEMINI_3_5_FLASH_OUTPUT_NANOS,
            thinking_nanos: GEMINI_3_5_FLASH_OUTPUT_NANOS,
        },
        "gemini-3.5-flash-lite" => Rates {
            input_nanos: GEMINI_3_5_FLASH_LITE_INPUT_NANOS,
            output_nanos: GEMINI_3_5_FLASH_LITE_OUTPUT_NANOS,
            thinking_nanos: GEMINI_3_5_FLASH_LITE_OUTPUT_NANOS,
        },
        "gemini-2.5-flash-lite" => Rates {
            input_nanos: GEMINI_2_5_FLASH_LITE_INPUT_NANOS,
            output_nanos: GEMINI_2_5_FLASH_LITE_OUTPUT_NANOS,
            thinking_nanos: GEMINI_2_5_FLASH_LITE_OUTPUT_NANOS,
        },
        "gemini-3.1-flash-image-preview" | "gemini-3.1-flash-image" => Rates {
            input_nanos: GEMINI_3_1_FLASH_IMAGE_INPUT_NANOS,
            output_nanos: GEMINI_3_1_FLASH_IMAGE_OUTPUT_NANOS,
            thinking_nanos: GEMINI_3_1_FLASH_IMAGE_THINKING_NANOS,
        },
        "gemini-3.1-flash-tts-preview" => Rates {
            input_nanos: GEMINI_3_1_FLASH_TTS_INPUT_NANOS,
            output_nanos: GEMINI_3_1_FLASH_TTS_OUTPUT_NANOS,
            thinking_nanos: GEMINI_3_1_FLASH_TTS_OUTPUT_NANOS,
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_model_usage_cannot_be_reported_as_free_generation() {
        let usage = UsageMetadata {
            prompt_token_count: 137,
            candidates_token_count: 59,
            thoughts_token_count: 11,
            total_token_count: 207,
        };
        assert_eq!(
            serde_json::to_value(priced("custom-unpriced-model", Some(&usage)))
                .expect("usage must encode"),
            serde_json::json!({"model": "custom-unpriced-model", "requests": 1, "input_tokens": 137, "output_tokens": 70, "total_tokens": 207, "cost": {"nanos": 0, "incomplete": true}}),
            "an unpriced model erased real usage or claimed a measured zero-dollar request"
        );
    }

    #[test]
    fn three_eight_flash_cannot_drop_input_output_or_thinking_costs() {
        let usage = UsageMetadata {
            prompt_token_count: 137,
            candidates_token_count: 29,
            thoughts_token_count: 43,
            total_token_count: 209,
        };
        let cost = priced("gemini-3.8-flash", Some(&usage));
        assert_eq!(
            (cost.model(), cost.requests(), cost.cost().nanos()),
            ("gemini-3.8-flash", 1, 372_750),
            "Gemini 3.8 Flash lost its model attribution or current Standard input, visible output, and thinking rates"
        );
    }

    #[test]
    fn three_seven_flash_uses_its_standard_text_rates() {
        let usage = UsageMetadata {
            prompt_token_count: 100,
            candidates_token_count: 20,
            thoughts_token_count: 30,
            total_token_count: 150,
        };
        assert_eq!(
            priced("gemini-3.7-flash", Some(&usage)).cost().nanos(),
            262_500,
            "Gemini 3.7 Flash pricing drifted from its current Standard rates"
        );
    }

    #[test]
    fn flash_text_usage_prices_input_output_and_thinking_tokens() {
        let usage = UsageMetadata {
            prompt_token_count: 100,
            candidates_token_count: 20,
            thoughts_token_count: 30,
            total_token_count: 150,
        };
        assert_eq!(
            priced("gemini-3.6-flash", Some(&usage)).cost().nanos(),
            525_000,
            "flash text pricing must apply the current paid-tier rates to visible and thinking tokens"
        );
    }

    #[test]
    fn image_usage_prices_generated_image_tokens() {
        let usage = UsageMetadata {
            prompt_token_count: 200,
            candidates_token_count: 1_120,
            thoughts_token_count: 500,
            total_token_count: 1_820,
        };
        assert_eq!(
            priced("gemini-3.1-flash-image", Some(&usage))
                .cost()
                .dollars(),
            "$.0688",
            "image pricing must price generated image and thinking tokens at their distinct rates"
        );
    }

    #[test]
    fn flash_lite_usage_prices_low_cost_scene_features() {
        let usage = UsageMetadata {
            prompt_token_count: 1_000,
            candidates_token_count: 200,
            thoughts_token_count: 300,
            total_token_count: 1_500,
        };
        assert_eq!(
            priced("gemini-3.5-flash-lite", Some(&usage)).cost().nanos(),
            1_550_000,
            "Flash Lite feature pricing drifted from the current paid-tier rates"
        );
    }

    #[test]
    fn two_five_flash_lite_prices_multimodal_recall_review() {
        let usage = UsageMetadata {
            prompt_token_count: 536,
            candidates_token_count: 76,
            thoughts_token_count: 0,
            total_token_count: 612,
        };
        assert_eq!(
            priced("gemini-2.5-flash-lite", Some(&usage)).cost().nanos(),
            84_000,
            "Gemini 2.5 Flash-Lite recall pricing drifted from the paid-tier rates"
        );
    }

    #[test]
    fn tts_usage_prices_audio_output_tokens() {
        let usage = UsageMetadata {
            prompt_token_count: 300,
            candidates_token_count: 500,
            thoughts_token_count: 0,
            total_token_count: 800,
        };
        assert_eq!(
            priced("gemini-3.1-flash-tts-preview", Some(&usage))
                .cost()
                .nanos(),
            10_300_000,
            "tts pricing must apply text input and audio output token rates"
        );
    }

    #[test]
    fn missing_usage_metadata_preserves_the_request_without_inventing_a_price() {
        assert_eq!(
            serde_json::to_value(priced("gemini-3.6-flash", None))
                .expect("unreported request must encode"),
            serde_json::json!({"model":"gemini-3.6-flash", "requests":1, "input_tokens":0, "output_tokens":0, "total_tokens":0, "cost":{"nanos":0,"incomplete":true}, "missing_usage":true}),
            "missing Gemini usage metadata erased a real request or claimed a zero-dollar price"
        );
    }
}
