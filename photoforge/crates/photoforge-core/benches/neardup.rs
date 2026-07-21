//! Naive all-pairs vs. BK-tree near-duplicate grouping over synthetic hashes.
//!
//! Run with `cargo bench -p photoforge-core --bench neardup`. Prints the
//! speedup ratio per size after the criterion runs.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use photoforge_core::dedupe::{group_near, NearMethod};
use std::hint::black_box;
use std::time::Instant;

/// Deterministic xorshift64* — no rand dependency needed for synthetic data.
fn hashes(n: usize) -> Vec<u64> {
    let mut x = 0x243f6a8885a308d3u64;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        out.push(x.wrapping_mul(0x2545f4914f6cdd1d));
        // Plant near-duplicates so grouping does real work.
        if i % 20 == 0 {
            let flipped = *out.last().unwrap() ^ (1u64 << (i % 64));
            out.push(flipped);
        }
    }
    out.truncate(n);
    out
}

fn bench_neardup(c: &mut Criterion) {
    let k = 5;
    let mut group = c.benchmark_group("near_duplicates");
    group.sample_size(10);

    for &n in &[1_000usize, 10_000, 50_000] {
        let data = hashes(n);
        // Naive at 50k is ~1.25B comparisons per iteration — skip it in the
        // criterion loop and report it once below instead.
        if n <= 10_000 {
            group.bench_with_input(BenchmarkId::new("naive", n), &data, |b, d| {
                b.iter(|| group_near(black_box(d), k, NearMethod::Naive))
            });
        }
        group.bench_with_input(BenchmarkId::new("bktree", n), &data, |b, d| {
            b.iter(|| group_near(black_box(d), k, NearMethod::BkTree))
        });
    }
    group.finish();

    // One-shot speedup ratios (including the 50k naive run criterion skips).
    println!("\nspeedup (naive / bktree), single run:");
    for &n in &[1_000usize, 10_000, 50_000] {
        let data = hashes(n);
        let t0 = Instant::now();
        let a = group_near(&data, k, NearMethod::Naive);
        let naive = t0.elapsed();
        let t1 = Instant::now();
        let b = group_near(&data, k, NearMethod::BkTree);
        let bk = t1.elapsed();
        assert_eq!(a.len(), b.len());
        println!(
            "  n={n:>6}: naive {naive:>10.2?}  bktree {bk:>10.2?}  speedup {:.1}x",
            naive.as_secs_f64() / bk.as_secs_f64()
        );
    }
}

criterion_group!(benches, bench_neardup);
criterion_main!(benches);
