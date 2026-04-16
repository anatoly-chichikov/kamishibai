use image::GrayImage;

/// Detect white borders and gutters in one grayscale image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BorderDetector {
    width: usize,
    brightness: u8,
    margin: usize,
}

impl BorderDetector {
    /// Create one border detector.
    pub fn new(width: usize, brightness: u8, margin: usize) -> Self {
        Self {
            width,
            brightness,
            margin,
        }
    }

    /// Return whether one white horizontal gutter exists.
    pub fn gutter(&self, image: &GrayImage) -> bool {
        if self.width == 0 {
            return true;
        }
        let mut run = 0usize;
        for y in 0..image.height() {
            if row(image, y) > f64::from(self.brightness) {
                run += 1;
                if run >= self.width {
                    return true;
                }
            } else {
                run = 0;
            }
        }
        false
    }

    /// Return the edge names that fail the white border check.
    pub fn borders(&self, image: &GrayImage) -> Vec<String> {
        let mut failed = Vec::new();
        let rows = self.margin.min(image.height() as usize) as u32;
        let cols = self.margin.min(image.width() as usize) as u32;
        if rows > 0 && band(image, 0, 0, image.width(), rows) <= f64::from(self.brightness) {
            failed.push(String::from("top"));
        }
        if rows > 0
            && band(image, 0, image.height() - rows, image.width(), rows)
                <= f64::from(self.brightness)
        {
            failed.push(String::from("bottom"));
        }
        if cols > 0 && band(image, 0, 0, cols, image.height()) <= f64::from(self.brightness) {
            failed.push(String::from("left"));
        }
        if cols > 0
            && band(image, image.width() - cols, 0, cols, image.height())
                <= f64::from(self.brightness)
        {
            failed.push(String::from("right"));
        }
        failed
    }
}

fn row(image: &GrayImage, y: u32) -> f64 {
    band(image, 0, y, image.width(), 1)
}

fn band(image: &GrayImage, x: u32, y: u32, width: u32, height: u32) -> f64 {
    let mut total = 0u64;
    let mut count = 0u64;
    for ypos in y..(y + height) {
        for xpos in x..(x + width) {
            total += u64::from(image.get_pixel(xpos, ypos)[0]);
            count += 1;
        }
    }
    if count == 0 {
        return 0.0;
    }
    total as f64 / count as f64
}
