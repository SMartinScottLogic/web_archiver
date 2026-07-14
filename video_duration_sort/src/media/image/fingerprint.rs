use anyhow::Result;
use image::{DynamicImage, GrayImage, Luma, imageops::FilterType};

const LARGE_SIZE: u32 = 160;
const FINAL_SIZE: u32 = 16;

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    pub bits: [u64; 4],
}

#[derive(Clone, Debug)]
pub struct ImageRecord {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fingerprint: Fingerprint,
}

impl Fingerprint {
    pub fn distance(&self, other: &Self) -> u32 {
        self.bits
            .iter()
            .zip(other.bits.iter())
            .map(|(a, b)| (a ^ b).count_ones())
            .sum()
    }

    pub fn similarity(&self, other: &Self) -> f32 {
        1.0 - self.distance(other) as f32 / 256.0
    }
}

pub fn generate(img: DynamicImage) -> Result<Fingerprint> {
    let img = img
        .resize_exact(LARGE_SIZE, LARGE_SIZE, FilterType::Triangle)
        .grayscale();

    let gray = img.to_luma8();

    let blurred = blur(&gray);

    let normalized = normalize(&blurred);

    let reduced =
        image::imageops::resize(&normalized, FINAL_SIZE, FINAL_SIZE, FilterType::Triangle);

    Ok(threshold_bitmap(&reduced))
}

fn blur(img: &GrayImage) -> GrayImage {
    image::imageops::blur(img, 8.0)
}

fn normalize(img: &GrayImage) -> GrayImage {
    let mut min = 255u8;
    let mut max = 0u8;

    for p in img.pixels() {
        let v = p[0];
        min = min.min(v);
        max = max.max(v);
    }

    if min == max {
        return img.clone();
    }

    let scale = 255.0 / (max - min) as f32;

    let mut out = img.clone();

    for pixel in out.pixels_mut() {
        let v = pixel[0];
        let nv = ((v - min) as f32 * scale).round().clamp(0.0, 255.0);

        *pixel = Luma([nv as u8]);
    }

    out
}

fn threshold_bitmap(img: &GrayImage) -> Fingerprint {
    let mean = img.pixels().map(|p| p[0] as u64).sum::<u64>() as f32 / 256.0;

    let mut bits = [0u64; 4];

    for (idx, pixel) in img.pixels().enumerate() {
        if pixel[0] as f32 > mean {
            let bucket = idx / 64;
            let offset = idx % 64;

            bits[bucket] |= 1u64 << offset;
        }
    }

    Fingerprint { bits }
}
