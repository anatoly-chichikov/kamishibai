//! Deterministic scale-aware crops for downstream literal-writing review.

use std::io::Cursor;

use anyhow::{Result, bail};
use image::{DynamicImage, GenericImageView, ImageFormat, imageops::FilterType};

const GRID: u32 = 3;
const TILE_NUMERATOR: u32 = 3;
const TILE_DENOMINATOR: u32 = 8;
const OUTPUT_EDGE: u32 = 1024;

/// Encode nine overlapping enlarged PNG crops from one candidate illustration.
pub(crate) fn literal_zoom_crops(encoded: &[u8]) -> Result<Vec<Vec<u8>>> {
    let source = image::load_from_memory(encoded)?.into_luma8();
    let (width, height) = source.dimensions();
    if width != height || width < TILE_DENOMINATOR {
        bail!("literal zoom review requires a nontrivial square source image");
    }
    let edge = width
        .checked_mul(TILE_NUMERATOR)
        .and_then(|value| value.checked_div(TILE_DENOMINATOR))
        .filter(|value| *value > 0)
        .ok_or_else(|| anyhow::anyhow!("literal zoom crop geometry overflow"))?;
    let remainder = width
        .checked_sub(edge)
        .ok_or_else(|| anyhow::anyhow!("literal zoom crop exceeds the source image"))?;
    let origins = [0, remainder / 2, remainder];
    let mut crops = Vec::with_capacity(usize::try_from(GRID * GRID)?);
    for y in origins {
        for x in origins {
            let crop = source.view(x, y, edge, edge).to_image();
            let enlarged =
                image::imageops::resize(&crop, OUTPUT_EDGE, OUTPUT_EDGE, FilterType::Lanczos3);
            let mut encoded = Cursor::new(Vec::new());
            DynamicImage::ImageLuma8(enlarged).write_to(&mut encoded, ImageFormat::Png)?;
            crops.push(encoded.into_inner());
        }
    }
    Ok(crops)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{DynamicImage, GrayImage, ImageFormat, Luma};

    use super::literal_zoom_crops;

    #[test]
    fn zoom_crops_cover_the_source_in_row_major_order_at_enlarged_scale() {
        let mut source = GrayImage::from_pixel(1024, 1024, Luma([255]));
        let centers = [192, 512, 832];
        let values = [16, 32, 48, 64, 80, 96, 112, 128, 144];
        for (index, value) in values.into_iter().enumerate() {
            let row = index / 3;
            let column = index % 3;
            for y in centers[row] - 8..centers[row] + 8 {
                for x in centers[column] - 8..centers[column] + 8 {
                    source.put_pixel(x, y, Luma([value]));
                }
            }
        }
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(source)
            .write_to(&mut encoded, ImageFormat::Png)
            .expect("synthetic source must encode");
        let crops = literal_zoom_crops(encoded.get_ref()).expect("zoom crops must encode");
        let decoded = crops
            .iter()
            .map(|crop| {
                image::load_from_memory(crop)
                    .expect("zoom crop must decode")
                    .into_luma8()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            (
                decoded.len(),
                decoded
                    .iter()
                    .map(image::GenericImageView::dimensions)
                    .collect::<Vec<_>>(),
                decoded
                    .iter()
                    .map(|crop| crop.get_pixel(512, 512).0[0])
                    .collect::<Vec<_>>(),
            ),
            (9, vec![(1024, 1024); 9], values.to_vec()),
            "zoom crops lost row-major coverage, output scale, or source-region identity"
        );
    }

    #[test]
    fn adjacent_zoom_crops_preserve_the_sixty_four_pixel_overlap() {
        let mut source = GrayImage::from_pixel(1024, 1024, Luma([255]));
        for y in 188..196 {
            for x in 348..356 {
                source.put_pixel(x, y, Luma([0]));
            }
        }
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(source)
            .write_to(&mut encoded, ImageFormat::Png)
            .expect("overlap source must encode");
        let crops = literal_zoom_crops(encoded.get_ref()).expect("overlap crops must encode");
        let pixels = crops
            .iter()
            .take(3)
            .enumerate()
            .map(|(index, crop)| {
                let image = image::load_from_memory(crop)
                    .expect("overlap crop must decode")
                    .into_luma8();
                let x = match index {
                    0 => 939,
                    1 => 85,
                    _ => 1,
                };
                image.get_pixel(x, 512).0[0]
            })
            .collect::<Vec<_>>();
        assert_eq!(
            pixels,
            vec![0, 0, 255],
            "adjacent zoom crops lost the approved sixty-four-pixel source overlap"
        );
    }

    #[test]
    fn tiny_lower_center_pseudo_rows_are_enlarged_in_crop_eight() {
        let mut source = GrayImage::from_pixel(1024, 1024, Luma([255]));
        for y in [674, 679, 684, 689] {
            for x in 430..462 {
                source.put_pixel(x, y, Luma([0]));
            }
        }
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(source)
            .write_to(&mut encoded, ImageFormat::Png)
            .expect("tiny-row source must encode");
        let crops = literal_zoom_crops(encoded.get_ref()).expect("tiny-row crops must encode");
        let crop = image::load_from_memory(&crops[7])
            .expect("crop eight must decode")
            .into_luma8();
        let dark = (80..400)
            .flat_map(|y| (200..500).map(move |x| (x, y)))
            .filter(|(x, y)| crop.get_pixel(*x, *y).0[0] < 32)
            .count();
        assert!(
            dark >= 256,
            "crop eight did not enlarge the tiny lower-center pseudo-writing rows"
        );
    }
}
