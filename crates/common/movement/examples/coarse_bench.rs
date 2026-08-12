//! A reproducible, deliberately small cost probe for the coarse router.
//!
//! ```sh
//! cargo run -p openshard-movement --example coarse_bench --release
//! ```
//!
//! It is an example rather than an asserted test: elapsed time is a property of
//! the host, while both returned route lengths are properties the output makes
//! visible for a human comparison.

use std::time::Instant;

use openshard_movement::{NavigationGraph, OpenWorld, find_long_path, find_path};
use openshard_protocol::world::Point;

const WIDTH: u16 = 1024;
const HEIGHT: u16 = 1024;
const FROM: Point = Point::new(1, 1, 0);
const TO: Point = Point::new(WIDTH - 2, HEIGHT - 2, 0);
const SAMPLES: usize = 25;

fn percentile(samples: &mut [std::time::Duration], percentile: usize) -> std::time::Duration {
    samples.sort_unstable();
    let index = (samples.len() * percentile).div_ceil(100).saturating_sub(1);
    samples[index]
}

fn main() {
    let built_at = Instant::now();
    let router = NavigationGraph::build(&OpenWorld, u32::from(WIDTH), u32::from(HEIGHT))
        .expect("the synthetic facet fits Point's coordinate space");
    let built = built_at.elapsed();

    let flat_at = Instant::now();
    let flat = find_path(&OpenWorld, FROM, TO, usize::from(WIDTH) * usize::from(HEIGHT))
        .expect("open ground has a flat route");
    let flat_elapsed = flat_at.elapsed();

    let mut coarse_samples = Vec::with_capacity(SAMPLES);
    let mut coarse_steps = None;
    for _ in 0..SAMPLES {
        let coarse_at = Instant::now();
        let coarse = find_long_path(&OpenWorld, &OpenWorld, &router, FROM, TO, 600)
            .expect("the coarse corridor has bounded exact hops");
        coarse_steps = Some(coarse.len());
        coarse_samples.push(coarse_at.elapsed());
    }
    let coarse_p95 = percentile(&mut coarse_samples, 95);
    let coarse_worst = coarse_samples.last().copied().expect("samples are non-empty");

    println!(
        "{WIDTH}x{HEIGHT}: build {built:?}; flat {} steps in {flat_elapsed:?}; coarse {} steps, p95 {coarse_p95:?}, worst {coarse_worst:?} ({SAMPLES} samples)",
        flat.len(),
        coarse_steps.expect("samples are non-empty"),
    );
}
