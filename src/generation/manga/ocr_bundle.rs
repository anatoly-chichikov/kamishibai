//! PaddleOCR model routing and delivery for the Rust runtime.

use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use ocr_rs::{OcrEngine, OcrEngineConfig};
use reqwest::blocking::Client;

use crate::generation::artifact_cache::Cache;
use crate::languages::OcrModel;

const CACHE: &str = "ocr-models";
const URL: &str = "https://raw.githubusercontent.com/zibo-chen/rust-paddle-ocr/next/models";
const DET: &str = "PP-OCRv5_mobile_det.mnn";

/// Return the recognition model filename for one typed bundle.
fn model_name(model: OcrModel) -> &'static str {
    match model {
        OcrModel::Cyrillic => "cyrillic_PP-OCRv5_mobile_rec_infer.mnn",
        OcrModel::Default => "PP-OCRv5_mobile_rec.mnn",
        OcrModel::El => "el_PP-OCRv5_mobile_rec_infer.mnn",
        OcrModel::En => "en_PP-OCRv5_mobile_rec_infer.mnn",
        OcrModel::Latin => "latin_PP-OCRv5_mobile_rec_infer.mnn",
        OcrModel::Korean => "korean_PP-OCRv5_mobile_rec_infer.mnn",
        OcrModel::Arabic => "arabic_PP-OCRv5_mobile_rec_infer.mnn",
        OcrModel::Devanagari => "devanagari_PP-OCRv5_mobile_rec_infer.mnn",
        OcrModel::Th => "th_PP-OCRv5_mobile_rec_infer.mnn",
    }
}

/// Return the charset filename for one typed bundle.
fn charset_name(model: OcrModel) -> &'static str {
    match model {
        OcrModel::Cyrillic => "ppocr_keys_cyrillic.txt",
        OcrModel::Default => "ppocr_keys_v5.txt",
        OcrModel::El => "ppocr_keys_el.txt",
        OcrModel::En => "ppocr_keys_en.txt",
        OcrModel::Latin => "ppocr_keys_latin.txt",
        OcrModel::Korean => "ppocr_keys_korean.txt",
        OcrModel::Arabic => "ppocr_keys_arabic.txt",
        OcrModel::Devanagari => "ppocr_keys_devanagari.txt",
        OcrModel::Th => "ppocr_keys_th.txt",
    }
}

/// Hold the resolved local model paths for one OCR engine.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Paths {
    charset: PathBuf,
    det: PathBuf,
    rec: PathBuf,
}

/// Create one OCR engine for the requested typed PP-OCRv5 bundle.
pub(crate) fn engine(model: OcrModel, root: &Path) -> Result<OcrEngine> {
    let paths = Catalog::new(root).paths(model)?;
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
    .map_err(|error| anyhow!("Failed to load OCR bundle for '{model:?}': {error}"))
}

/// Route one legacy OCR selection string to one OCR bundle.
pub(super) fn legacy_model(selection: &str) -> OcrModel {
    if matches(selection, &["chi_sim", "chi_tra", "chi", "zh"]) {
        return OcrModel::Default;
    }
    if matches(selection, &["ell", "el"]) {
        return OcrModel::El;
    }
    if matches(
        selection,
        &["rus", "ukr", "bel", "ru", "bg", "cyrillic", "eslav"],
    ) {
        return OcrModel::Cyrillic;
    }
    if matches(
        selection,
        &[
            "deu", "spa", "fra", "ita", "por", "nld", "dut", "lat", "de", "es", "nl",
        ],
    ) {
        return OcrModel::Latin;
    }
    if matches(selection, &["jpn"]) {
        return OcrModel::Default;
    }
    OcrModel::En
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

    /// Return the resolved local model paths for one typed bundle.
    fn paths(&self, model: OcrModel) -> Result<Paths> {
        Ok(Paths {
            charset: self.fetch(charset_name(model))?,
            det: self.fetch(DET)?,
            rec: self.fetch(model_name(model))?,
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
    use super::{charset_name, legacy_model, model_name};
    use crate::languages::OcrModel;

    /// German legacy OCR tokens route to the Latin PP-OCRv5 recognizer.
    #[test]
    fn german_legacy_ocr_tokens_route_to_the_latin_pp_ocrv5_recognizer() {
        assert_eq!(
            legacy_model("eng+deu"),
            OcrModel::Latin,
            "german legacy ocr tokens no longer route to the latin pp ocrv5 recognizer"
        );
    }

    /// Dutch legacy OCR tokens route to the Latin PP-OCRv5 recognizer.
    #[test]
    fn dutch_legacy_ocr_tokens_route_to_the_latin_pp_ocrv5_recognizer() {
        assert_eq!(
            legacy_model("eng+nld"),
            OcrModel::Latin,
            "dutch legacy ocr tokens no longer route to the latin pp ocrv5 recognizer"
        );
    }

    /// Greek legacy OCR tokens route to the Greek PP-OCRv5 recognizer.
    #[test]
    fn greek_legacy_ocr_tokens_route_to_the_greek_pp_ocrv5_recognizer() {
        assert_eq!(
            legacy_model("eng+ell"),
            OcrModel::El,
            "greek legacy ocr tokens no longer route to the greek pp ocrv5 recognizer"
        );
    }

    /// Russian legacy OCR tokens route to the Cyrillic PP-OCRv5 recognizer.
    #[test]
    fn russian_legacy_ocr_tokens_route_to_the_cyrillic_pp_ocrv5_recognizer() {
        assert_eq!(
            legacy_model("eng+rus"),
            OcrModel::Cyrillic,
            "russian legacy ocr tokens no longer route to the cyrillic pp ocrv5 recognizer"
        );
    }

    /// Auxiliary Japanese detection cannot replace the target script recognizer.
    #[test]
    fn auxiliary_japanese_detection_cannot_replace_the_target_script_recognizer() {
        assert_eq!(
            (legacy_model("eng+deu+jpn"), legacy_model("eng+rus+jpn")),
            (OcrModel::Latin, OcrModel::Cyrillic),
            "auxiliary Japanese detection replaced a target script recognizer"
        );
    }

    /// Chinese legacy OCR tokens route to the shared multilingual PP-OCRv5 recognizer.
    #[test]
    fn chinese_legacy_ocr_tokens_route_to_the_shared_multilingual_pp_ocrv5_recognizer() {
        assert_eq!(
            legacy_model("eng+chi_sim"),
            OcrModel::Default,
            "chinese legacy ocr tokens no longer route to the shared multilingual pp ocrv5 recognizer"
        );
    }

    /// Unknown OCR tokens still fall back to the English PP-OCRv5 recognizer.
    #[test]
    fn unknown_ocr_tokens_still_fall_back_to_the_english_pp_ocrv5_recognizer() {
        assert_eq!(
            legacy_model("osd"),
            OcrModel::En,
            "unknown ocr tokens no longer fall back to the english pp ocrv5 recognizer"
        );
    }

    /// New script bundles resolve to the exact upstream model and charset assets.
    #[test]
    fn new_script_bundles_resolve_to_the_upstream_pp_ocrv5_assets() {
        assert_eq!(
            [
                OcrModel::Korean,
                OcrModel::Arabic,
                OcrModel::Devanagari,
                OcrModel::Th,
            ]
            .map(|model| (model_name(model), charset_name(model))),
            [
                (
                    "korean_PP-OCRv5_mobile_rec_infer.mnn",
                    "ppocr_keys_korean.txt"
                ),
                (
                    "arabic_PP-OCRv5_mobile_rec_infer.mnn",
                    "ppocr_keys_arabic.txt"
                ),
                (
                    "devanagari_PP-OCRv5_mobile_rec_infer.mnn",
                    "ppocr_keys_devanagari.txt"
                ),
                ("th_PP-OCRv5_mobile_rec_infer.mnn", "ppocr_keys_th.txt"),
            ],
            "a new script bundle no longer resolves to its upstream model and charset"
        );
    }
}
