//! Paid-tier Gemini request cost estimation from response usage metadata.

use crate::session::{CostRecord, GenerationCost};

use super::protocol::UsageMetadata;

const GEMINI_3_6_FLASH_INPUT_NANOS: u64 = 1_500;
const GEMINI_3_6_FLASH_OUTPUT_NANOS: u64 = 7_500;
const GEMINI_3_5_FLASH_INPUT_NANOS: u64 = 1_500;
const GEMINI_3_5_FLASH_OUTPUT_NANOS: u64 = 9_000;
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
    match usage {
        Some(usage) => rates(model).priced(usage, model),
        None => CostRecord::new(model, 0, 0, 0, 0, GenerationCost::zero()),
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

fn rates(model: &str) -> Rates {
    match model {
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
        _ => Rates {
            input_nanos: 0,
            output_nanos: 0,
            thinking_nanos: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn missing_usage_metadata_is_not_a_billable_record() {
        assert_eq!(
            priced("gemini-3.6-flash", None).requests(),
            0,
            "missing Gemini usage metadata must not be rendered as a zero-dollar request"
        );
    }
}
