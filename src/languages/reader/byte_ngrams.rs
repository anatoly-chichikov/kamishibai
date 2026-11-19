//! Inference for the selected columns of the langid-rs byte n-gram model.
use byteorder::{LittleEndian, ReadBytesExt};
use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::LazyLock;

struct Features {
    transitions: Vec<u16>,
    emitted: HashMap<u16, Vec<usize>>,
    count: usize,
}

struct Classifier {
    features: Features,
    codes: Vec<String>,
    weights: Vec<f32>,
    priors: Vec<f32>,
}

impl Classifier {
    fn read(mut input: impl Read) -> io::Result<Self> {
        let features = number(&mut input)?;
        let languages = number(&mut input)?;
        let weights = (0..features * languages)
            .map(|_| input.read_f32::<LittleEndian>())
            .collect::<io::Result<_>>()?;
        let priors = (0..languages)
            .map(|_| input.read_f32::<LittleEndian>())
            .collect::<io::Result<_>>()?;
        let states = number(&mut input)?;
        let transitions = (0..states)
            .map(|_| input.read_u16::<LittleEndian>())
            .collect::<io::Result<_>>()?;
        let codes = (0..languages)
            .map(|_| {
                let mut code = [0; 2];
                input.read_exact(&mut code)?;
                String::from_utf8(code.to_vec()).map_err(io::Error::other)
            })
            .collect::<io::Result<_>>()?;
        let outputs = number(&mut input)?;
        let mut emitted = HashMap::with_capacity(outputs);
        for _ in 0..outputs {
            let state = u16::try_from(number(&mut input)?).map_err(io::Error::other)?;
            let length = number(&mut input)?;
            let ids = (0..length)
                .map(|_| number(&mut input))
                .collect::<io::Result<_>>()?;
            emitted.insert(state, ids);
        }
        Ok(Self {
            features: Features {
                transitions,
                emitted,
                count: features,
            },
            codes,
            weights,
            priors,
        })
    }

    fn rank(&self, text: &str) -> Vec<(&str, f32)> {
        let mut occurrences = vec![0u16; self.features.count];
        let mut state = 0;
        for byte in text.bytes() {
            state = self.features.transitions[usize::from(state) * 256 + usize::from(byte)];
            if let Some(ids) = self.features.emitted.get(&state) {
                for &id in ids {
                    occurrences[id] = occurrences[id].saturating_add(1);
                }
            }
        }
        let mut scores = vec![0.0; self.codes.len()];
        for (count, weights) in occurrences
            .into_iter()
            .zip(self.weights.chunks_exact(self.codes.len()))
        {
            if count != 0 {
                for (score, weight) in scores.iter_mut().zip(weights) {
                    *score += f32::from(count) * weight;
                }
            }
        }
        let mut ranking: Vec<_> = self
            .codes
            .iter()
            .zip(scores)
            .zip(&self.priors)
            .map(|((code, score), prior)| (code.as_str(), score + prior))
            .collect();
        ranking.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranking
    }
}

fn number(input: &mut impl Read) -> io::Result<usize> {
    usize::try_from(input.read_u32::<LittleEndian>()?).map_err(io::Error::other)
}

/// Rank the ambiguous reader languages using unmodified bytes, including diacritics.
pub(super) fn rank(text: &str) -> Vec<(&'static str, f32)> {
    static MODEL: LazyLock<Classifier> = LazyLock::new(|| {
        Classifier::read(GzDecoder::new(
            include_bytes!("data/byte-ngrams.gz").as_slice(),
        ))
        .expect("invariant: the embedded byte n-grams must decode")
    });
    MODEL.rank(text)
}
