//! Chroma acceptance for strictly monochrome manga output.

use image::{DynamicImage, Pixel, imageops::FilterType};

const SAMPLE_EDGE: u32 = 256;
const NOISE_CEILING: u32 = 7;
const REJECT_SCALE: u64 = 10;

/// Return whether an image contains a production-significant amount of color.
pub(super) fn color_detected(image: &DynamicImage) -> bool {
    let sampled = image
        .resize(SAMPLE_EDGE, SAMPLE_EDGE, FilterType::Triangle)
        .into_rgb8();
    let total = u64::from(sampled.width()).saturating_mul(u64::from(sampled.height()));
    let colored = sampled
        .pixels()
        .filter(|pixel| chroma(pixel.channels()) > NOISE_CEILING)
        .count();
    u64::try_from(colored)
        .unwrap_or(u64::MAX)
        .saturating_mul(REJECT_SCALE)
        >= total
}

fn chroma(channels: &[u8]) -> u32 {
    let red = i32::from(channels[0]);
    let green = i32::from(channels[1]);
    let blue = i32::from(channels[2]);
    let blue_difference = (-43 * red - 85 * green + 128 * blue) / 256;
    let red_difference = (128 * red - 107 * green - 21 * blue) / 256;
    blue_difference
        .unsigned_abs()
        .max(red_difference.unsigned_abs())
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, Rgb, RgbImage};

    use super::color_detected;

    fn fixture(bytes: &[u8]) -> DynamicImage {
        image::load_from_memory(bytes).expect("monochrome calibration fixture must decode")
    }

    fn boundary(colored: usize) -> DynamicImage {
        let mut image = RgbImage::from_pixel(256, 256, Rgb([128, 128, 128]));
        for pixel in image.pixels_mut().take(colored) {
            *pixel = Rgb([128, 128, 144]);
        }
        DynamicImage::ImageRgb8(image)
    }

    /// Mandatory archived colored images are rejected before later gates.
    #[test]
    fn archived_colored_images_are_rejected() {
        let detected = [
            include_bytes!("../../../tests/fixtures/monochrome/color-linger.jpg").as_slice(),
            include_bytes!("../../../tests/fixtures/monochrome/color-water.jpg").as_slice(),
        ]
        .map(|bytes| color_detected(&fixture(bytes)));
        assert_eq!(
            detected, [true; 2],
            "an archived colored production image passed the monochrome gate"
        );
    }

    /// Confirmed archived monochrome images pass despite JPEG and screentone detail.
    #[test]
    fn archived_monochrome_images_pass() {
        let detected = [
            include_bytes!("../../../tests/fixtures/monochrome/mono-cut-through.jpg").as_slice(),
            include_bytes!("../../../tests/fixtures/monochrome/mono-adversarial.jpg").as_slice(),
            include_bytes!("../../../tests/fixtures/monochrome/mono-deed.jpg").as_slice(),
        ]
        .map(|bytes| color_detected(&fixture(bytes)));
        assert_eq!(
            detected, [false; 3],
            "a confirmed monochrome production image was rejected as colored"
        );
    }

    /// Chroma at the JPEG-noise ceiling remains admissible everywhere.
    #[test]
    fn amplitude_seven_is_not_color() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(256, 256, Rgb([128, 128, 142])));
        assert!(
            !color_detected(&image),
            "the calibrated JPEG-noise ceiling was classified as color"
        );
    }

    /// The ten-percent colored-pixel boundary is exact.
    #[test]
    fn colored_fraction_rejects_at_ten_percent() {
        assert_eq!(
            [6_553, 6_554].map(|colored| color_detected(&boundary(colored))),
            [false, true],
            "the production colored-pixel boundary drifted away from ten percent"
        );
    }
}
