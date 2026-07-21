//! PaddleOCR model routing and delivery for the Rust runtime.

use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use ocr_rs::{OcrEngine, OcrEngineConfig};
use reqwest::blocking::Client;

use crate::generation::artifact_cache::Cache;

const CACHE: &str = "ocr-models";
const URL: &str = "https://raw.githubusercontent.com/zibo-chen/rust-paddle-ocr/next/models";
const DET: &str = "PP-OCRv5_mobile_det.mnn";

/// Route one legacy OCR token string to one PP-OCRv5 model bundle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Bundle {
    Cyrillic,
    Default,
    El,
    En,
    Latin,
}

impl Bundle {
    /// Return the recognition model filename for one bundle.
    fn model(self) -> &'static str {
        match self {
            Self::Cyrillic => "cyrillic_PP-OCRv5_mobile_rec_infer.mnn",
            Self::Default => "PP-OCRv5_mobile_rec.mnn",
            Self::El => "el_PP-OCRv5_mobile_rec_infer.mnn",
            Self::En => "en_PP-OCRv5_mobile_rec_infer.mnn",
            Self::Latin => "latin_PP-OCRv5_mobile_rec_infer.mnn",
        }
    }

    /// Return the charset filename for one bundle.
    fn charset(self) -> &'static str {
        match self {
            Self::Cyrillic => "ppocr_keys_cyrillic.txt",
            Self::Default => "ppocr_keys_v5.txt",
            Self::El => "ppocr_keys_el.txt",
            Self::En => "ppocr_keys_en.txt",
            Self::Latin => "ppocr_keys_latin.txt",
        }
    }
}

/// Hold the resolved local model paths for one OCR engine.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Paths {
    charset: PathBuf,
    det: PathBuf,
    rec: PathBuf,
}

/// Create one OCR engine for the requested legacy OCR selection string.
pub(crate) fn engine(selection: &str, root: &Path) -> Result<OcrEngine> {
    let paths = Catalog::new(root).paths(selection)?;
    OcrEngine::new(
        paths.det.as_path(),
        paths.rec.as_path(),
        paths.charset.as_path(),
        Some(
            OcrEngineConfig::new()
                .with_parallel(false)
                .with_min_result_confidence(0.0),
        ),
    )
    .map_err(|error| anyhow!("Failed to load OCR bundle for '{}': {}", selection, error))
}

/// Route one legacy OCR selection string to one OCR bundle.
fn bundle(selection: &str) -> Bundle {
    if matches(selection, &["chi_sim", "chi_tra", "chi", "zh"]) {
        return Bundle::Default;
    }
    if matches(selection, &["ell", "el"]) {
        return Bundle::El;
    }
    if matches(
        selection,
        &["rus", "ukr", "bel", "ru", "bg", "cyrillic", "eslav"],
    ) {
        return Bundle::Cyrillic;
    }
    if matches(
        selection,
        &[
            "deu", "spa", "fra", "ita", "por", "nld", "dut", "lat", "de", "es", "nl",
        ],
    ) {
        return Bundle::Latin;
    }
    if matches(selection, &["jpn"]) {
        return Bundle::Default;
    }
    Bundle::En
}

/// Return whether one legacy OCR token string includes any item from the target set.
fn matches(selection: &str, targets: &[&str]) -> bool {
    selection
        .split('+')
        .map(|item| item.trim().to_ascii_lowercase())
        .any(|item| targets.iter().any(|target| item == *target))
}

/// Check whether one local model file already exists and is non-empty.
fn ready(path: &Path) -> bool {
    fs::metadata(path)
        .map(|item| item.is_file() && item.len() > 0)
        .unwrap_or(false)
}

/// Build one raw upstream model URL for the requested filename.
fn url(name: &str) -> String {
    format!("{URL}/{name}")
}

/// Keep one OCR model catalog rooted in the application cache.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Catalog {
    cache: Cache,
}

impl Catalog {
    /// Create one catalog rooted in the shared cache directory.
    fn new(root: &Path) -> Self {
        Self {
            cache: Cache::new(CACHE, root),
        }
    }

    /// Return the resolved local model paths for one legacy selection string.
    fn paths(&self, selection: &str) -> Result<Paths> {
        let bundle = bundle(selection);
        Ok(Paths {
            charset: self.fetch(bundle.charset())?,
            det: self.fetch(DET)?,
            rec: self.fetch(bundle.model())?,
        })
    }

    /// Return the local path for one ensured model asset.
    fn fetch(&self, name: &str) -> Result<PathBuf> {
        let path = self.cache.filepath(name)?;
        if ready(path.as_path()) {
            return Ok(path);
        }
        let staged = self.cache.stage(".part")?;
        let result = self
            .write(url(name).as_str(), staged.as_path())
            .and_then(|_| {
                if ready(staged.as_path()) {
                    return self.cache.commit(staged.as_path(), name);
                }
                Err(anyhow!("Downloaded OCR asset '{}' is empty", name))
            });
        if result.is_err() {
            let _ = fs::remove_file(staged.as_path());
        }
        result?;
        Ok(path)
    }

    /// Download one upstream model asset into one staged local file.
    fn write(&self, url: &str, path: &Path) -> Result<()> {
        let mut response = Client::new().get(url).send()?.error_for_status()?;
        let mut writer = BufWriter::new(fs::File::create(path)?);
        response.copy_to(&mut writer)?;
        writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Bundle, bundle};

    /// German legacy OCR tokens route to the Latin PP-OCRv5 recognizer.
    #[test]
    fn german_legacy_ocr_tokens_route_to_the_latin_pp_ocrv5_recognizer() {
        assert_eq!(
            bundle("eng+deu"),
            Bundle::Latin,
            "german legacy ocr tokens no longer route to the latin pp ocrv5 recognizer"
        );
    }

    /// Dutch legacy OCR tokens route to the Latin PP-OCRv5 recognizer.
    #[test]
    fn dutch_legacy_ocr_tokens_route_to_the_latin_pp_ocrv5_recognizer() {
        assert_eq!(
            bundle("eng+nld"),
            Bundle::Latin,
            "dutch legacy ocr tokens no longer route to the latin pp ocrv5 recognizer"
        );
    }

    /// Greek legacy OCR tokens route to the Greek PP-OCRv5 recognizer.
    #[test]
    fn greek_legacy_ocr_tokens_route_to_the_greek_pp_ocrv5_recognizer() {
        assert_eq!(
            bundle("eng+ell"),
            Bundle::El,
            "greek legacy ocr tokens no longer route to the greek pp ocrv5 recognizer"
        );
    }

    /// Russian legacy OCR tokens route to the Cyrillic PP-OCRv5 recognizer.
    #[test]
    fn russian_legacy_ocr_tokens_route_to_the_cyrillic_pp_ocrv5_recognizer() {
        assert_eq!(
            bundle("eng+rus"),
            Bundle::Cyrillic,
            "russian legacy ocr tokens no longer route to the cyrillic pp ocrv5 recognizer"
        );
    }

    /// Auxiliary Japanese detection cannot replace the target script recognizer.
    #[test]
    fn auxiliary_japanese_detection_cannot_replace_the_target_script_recognizer() {
        assert_eq!(
            (bundle("eng+deu+jpn"), bundle("eng+rus+jpn")),
            (Bundle::Latin, Bundle::Cyrillic),
            "auxiliary Japanese detection replaced a target script recognizer"
        );
    }

    /// Chinese legacy OCR tokens route to the shared multilingual PP-OCRv5 recognizer.
    #[test]
    fn chinese_legacy_ocr_tokens_route_to_the_shared_multilingual_pp_ocrv5_recognizer() {
        assert_eq!(
            bundle("eng+chi_sim"),
            Bundle::Default,
            "chinese legacy ocr tokens no longer route to the shared multilingual pp ocrv5 recognizer"
        );
    }

    /// Unknown OCR tokens still fall back to the English PP-OCRv5 recognizer.
    #[test]
    fn unknown_ocr_tokens_still_fall_back_to_the_english_pp_ocrv5_recognizer() {
        assert_eq!(
            bundle("osd"),
            Bundle::En,
            "unknown ocr tokens no longer fall back to the english pp ocrv5 recognizer"
        );
    }
}
