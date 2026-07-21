use std::collections::VecDeque;

use image::GrayImage;

const COVERAGE_SCALE: u64 = 100;
const DARK_COVERAGE: u64 = 80;
const DARKNESS: u8 = 128;
const MINIMUM_REGION_SCALE: usize = 1_000;
const MINIMUM_REGION_SHARE: usize = 15;
const REGION_BRIGHTNESS_MARGIN: u8 = 20;
const RAIL_SEARCH: u32 = 3;
const WHITE_COVERAGE: u64 = 99;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Span {
    start: u32,
    end: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Scan {
    axis: Axis,
    positions: Span,
    cross: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RegionScan {
    width: usize,
    height: usize,
    length: usize,
    claimed: Vec<bool>,
}

/// Detect white borders and gutters in one grayscale image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BorderDetector {
    minimum: usize,
    maximum: usize,
    brightness: u8,
    margin: usize,
}

impl BorderDetector {
    /// Create one detector with an inclusive gutter-width range.
    pub fn new(minimum: usize, maximum: usize, brightness: u8, margin: usize) -> Self {
        Self {
            minimum,
            maximum,
            brightness,
            margin,
        }
    }

    /// Return whether one internal white horizontal or vertical gutter exists.
    pub fn gutter(&self, image: &GrayImage) -> bool {
        self.gutter_within(image, 0, 0, image.width(), image.height())
    }

    /// Return whether one white gutter splits the interior of one rectangular region.
    pub(crate) fn gutter_within(
        &self,
        image: &GrayImage,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> bool {
        if self.minimum == 0 {
            return true;
        }
        let margin = u32::try_from(self.margin).unwrap_or(u32::MAX);
        let region_left = x.min(image.width());
        let region_top = y.min(image.height());
        let region_right = x.saturating_add(width).min(image.width());
        let region_bottom = y.saturating_add(height).min(image.height());
        let left = region_left.saturating_add(margin).min(region_right);
        let top = region_top.saturating_add(margin).min(region_bottom);
        let right = region_right.saturating_sub(margin).max(region_left);
        let bottom = region_bottom.saturating_sub(margin).max(region_top);
        if left >= right || top >= bottom {
            return false;
        }
        self.gutter_on(
            image,
            Scan {
                axis: Axis::Horizontal,
                positions: Span {
                    start: top,
                    end: bottom,
                },
                cross: Span {
                    start: left,
                    end: right,
                },
            },
        ) || self.gutter_on(
            image,
            Scan {
                axis: Axis::Vertical,
                positions: Span {
                    start: left,
                    end: right,
                },
                cross: Span {
                    start: top,
                    end: bottom,
                },
            },
        )
    }

    /// Count large closed panel regions separated from the outer white gutter network.
    pub fn regions(&self, image: &GrayImage) -> usize {
        let Some(mut scan) = region_scan(image, self.brightness) else {
            return 0;
        };
        let mut pending = VecDeque::new();
        (0..scan.length)
            .filter(|index| {
                if scan.claimed[*index] {
                    return false;
                }
                let area = claim_region(
                    *index,
                    scan.width,
                    scan.height,
                    &mut scan.claimed,
                    &mut pending,
                    |_| {},
                );
                area.saturating_mul(MINIMUM_REGION_SCALE)
                    >= scan.length.saturating_mul(MINIMUM_REGION_SHARE)
            })
            .count()
    }

    /// Return one region count and the corresponding ids at selected image coordinates.
    pub(crate) fn region_measure(
        &self,
        image: &GrayImage,
        points: &[(u32, u32)],
    ) -> (usize, Vec<Option<usize>>) {
        let Some(mut scan) = region_scan(image, self.brightness) else {
            return (0, vec![None; points.len()]);
        };
        let indices = points
            .iter()
            .map(|(x, y)| {
                let x = usize::try_from(*x).ok()?;
                let y = usize::try_from(*y).ok()?;
                if x >= scan.width || y >= scan.height {
                    return None;
                }
                y.checked_mul(scan.width)?.checked_add(x)
            })
            .collect::<Vec<_>>();
        let mut ids = vec![None; points.len()];
        let mut pending = VecDeque::new();
        let mut count = 0usize;
        let mut members = Vec::new();
        for index in 0..scan.length {
            if scan.claimed[index] {
                continue;
            }
            members.clear();
            claim_region(
                index,
                scan.width,
                scan.height,
                &mut scan.claimed,
                &mut pending,
                |member| members.push(member),
            );
            if members.len().saturating_mul(MINIMUM_REGION_SCALE)
                < scan.length.saturating_mul(MINIMUM_REGION_SHARE)
            {
                continue;
            }
            for (point, target) in indices.iter().zip(ids.iter_mut()) {
                if point.is_some_and(|point| members.contains(&point)) {
                    *target = Some(count);
                }
            }
            count = count.saturating_add(1);
        }
        (count, ids)
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

    fn gutter_on(&self, image: &GrayImage, scan: Scan) -> bool {
        let mut run = scan.positions.start;
        let mut active = false;
        for position in scan.positions.start..scan.positions.end {
            if coverage(
                image,
                scan,
                position,
                |pixel| pixel >= self.brightness,
                WHITE_COVERAGE,
            ) {
                if !active {
                    run = position;
                    active = true;
                }
            } else if active {
                if self.valid_run(image, scan, run, position) {
                    return true;
                }
                active = false;
            }
        }
        false
    }

    fn valid_run(&self, image: &GrayImage, scan: Scan, start: u32, end: u32) -> bool {
        let length = usize::try_from(end - start).unwrap_or(usize::MAX);
        (self.minimum..=self.maximum).contains(&length)
            && rail(
                image,
                scan,
                Span {
                    start: start.saturating_sub(RAIL_SEARCH).max(scan.positions.start),
                    end: start,
                },
            )
            && rail(
                image,
                scan,
                Span {
                    start: end,
                    end: end.saturating_add(RAIL_SEARCH).min(scan.positions.end),
                },
            )
    }
}

fn region_scan(image: &GrayImage, brightness: u8) -> Option<RegionScan> {
    let width = usize::try_from(image.width()).ok()?;
    let height = usize::try_from(image.height()).ok()?;
    let length = width.checked_mul(height)?;
    if width == 0 || height == 0 || length != image.as_raw().len() {
        return None;
    }
    let white = image
        .as_raw()
        .iter()
        .map(|pixel| *pixel >= brightness.saturating_sub(REGION_BRIGHTNESS_MARGIN))
        .collect::<Vec<_>>();
    let mut claimed = vec![false; length];
    let mut pending = VecDeque::new();
    for x in 0..width {
        seed(x, &white, &mut claimed, &mut pending);
        seed(
            (height - 1).saturating_mul(width).saturating_add(x),
            &white,
            &mut claimed,
            &mut pending,
        );
    }
    for y in 0..height {
        seed(y.saturating_mul(width), &white, &mut claimed, &mut pending);
        seed(
            y.saturating_mul(width).saturating_add(width - 1),
            &white,
            &mut claimed,
            &mut pending,
        );
    }
    flood_white(width, height, &white, &mut claimed, &mut pending);
    Some(RegionScan {
        width,
        height,
        length,
        claimed,
    })
}

fn seed(index: usize, white: &[bool], claimed: &mut [bool], pending: &mut VecDeque<usize>) {
    if white[index] && !claimed[index] {
        claimed[index] = true;
        pending.push_back(index);
    }
}

fn flood_white(
    width: usize,
    height: usize,
    white: &[bool],
    claimed: &mut [bool],
    pending: &mut VecDeque<usize>,
) {
    while let Some(index) = pending.pop_front() {
        for neighbor in neighbors(index, width, height).into_iter().flatten() {
            if white[neighbor] && !claimed[neighbor] {
                claimed[neighbor] = true;
                pending.push_back(neighbor);
            }
        }
    }
}

fn claim_region<F>(
    start: usize,
    width: usize,
    height: usize,
    claimed: &mut [bool],
    pending: &mut VecDeque<usize>,
    mut visit: F,
) -> usize
where
    F: FnMut(usize),
{
    claimed[start] = true;
    pending.push_back(start);
    let mut area = 0usize;
    while let Some(index) = pending.pop_front() {
        area = area.saturating_add(1);
        visit(index);
        for neighbor in neighbors(index, width, height).into_iter().flatten() {
            if !claimed[neighbor] {
                claimed[neighbor] = true;
                pending.push_back(neighbor);
            }
        }
    }
    area
}

fn neighbors(index: usize, width: usize, height: usize) -> [Option<usize>; 4] {
    let x = index % width;
    let y = index / width;
    [
        (x > 0).then(|| index - 1),
        (x + 1 < width).then(|| index + 1),
        (y > 0).then(|| index - width),
        (y + 1 < height).then(|| index + width),
    ]
}

fn rail(image: &GrayImage, scan: Scan, span: Span) -> bool {
    (span.start..span.end).any(|position| {
        coverage(
            image,
            scan,
            position,
            |pixel| pixel <= DARKNESS,
            DARK_COVERAGE,
        )
    })
}

fn coverage<F>(image: &GrayImage, scan: Scan, position: u32, matches: F, required: u64) -> bool
where
    F: Fn(u8) -> bool,
{
    let matched = (scan.cross.start..scan.cross.end)
        .filter(|cross| {
            let (x, y) = match scan.axis {
                Axis::Horizontal => (*cross, position),
                Axis::Vertical => (position, *cross),
            };
            matches(image.get_pixel(x, y)[0])
        })
        .count();
    let total = scan.cross.end.saturating_sub(scan.cross.start);
    let matched = u64::try_from(matched).unwrap_or(u64::MAX);
    matched.saturating_mul(COVERAGE_SCALE) >= u64::from(total).saturating_mul(required)
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

#[cfg(test)]
mod tests {
    use image::{GrayImage, Luma};

    use super::BorderDetector;

    /// Region lookup rejects coordinates outside the image instead of wrapping rows.
    #[test]
    fn region_lookup_rejects_out_of_range_coordinates() {
        let image = GrayImage::from_pixel(16, 16, Luma([0]));
        assert_eq!(
            BorderDetector::new(2, 6, 240, 1)
                .region_measure(&image, &[(16, 0), (0, 16), (32, 1)])
                .1,
            vec![None, None, None],
            "region lookup wraps an out-of-range coordinate into another row"
        );
    }
}
