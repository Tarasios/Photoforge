//! dHash throughput benchmark: hash 1000 synthetic 256x256 images.
//!
//! Images are pre-generated in memory so the benchmark isolates the hash
//! (resize + compare) from disk I/O and format decoding.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use image::{DynamicImage, Rgb, RgbImage};
use photoforge_core::dhash::dhash_image;
use std::hint::black_box;

fn synthetic(seed: u32) -> DynamicImage {
    let mut img = RgbImage::new(256, 256);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let v = ((x * 7 + y * 13 + seed * 31) % 256) as u8;
        *p = Rgb([v, v.wrapping_mul(3), v.wrapping_add(97)]);
    }
    DynamicImage::ImageRgb8(img)
}

fn bench_dhash(c: &mut Criterion) {
    let images: Vec<DynamicImage> = (0..1000).map(synthetic).collect();
    let mut group = c.benchmark_group("dhash");
    group.throughput(Throughput::Elements(images.len() as u64));
    group.sample_size(10);
    group.bench_function("1000_images_256px", |b| {
        b.iter(|| {
            let mut acc = 0u64;
            for img in &images {
                acc ^= dhash_image(black_box(img));
            }
            acc
        })
    });
    group.finish();
}

criterion_group!(benches, bench_dhash);
criterion_main!(benches);
