//! Benchmark harness: keep the current decoder unchanged and record each
//! rejected shot instead of aborting the entire Monte Carlo experiment.
use microblossom_toolchain::decoding::{ShotBatch, decode_batch, prepare_decoder};
use microblossom_toolchain::window::build_window_graphs;
use microblossom_toolchain::{Dem, convert};
use serde_json::json;
use std::{env, fs, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 4 {
        return Err("usage: window_accuracy INPUT.ir.json SHOTS.json OUTPUT.json".into());
    }
    let start = Instant::now();
    let dem: Dem = serde_json::from_str(&fs::read_to_string(&args[1])?)?;
    let graph = convert(dem, 100.0, true)?;
    let windows = build_window_graphs(&graph)?;
    let decoder = prepare_decoder(&graph, &windows)?;
    let initialization_seconds = start.elapsed().as_secs_f64();
    eprintln!("initialized in {initialization_seconds:.3}s");
    let input: ShotBatch = serde_json::from_str(&fs::read_to_string(&args[2])?)?;
    let start = Instant::now();
    let mut rows = Vec::new();
    for (index, shot) in input.shots.into_iter().enumerate() {
        let one = ShotBatch { shots: vec![shot] };
        // This is the production CLI's decode/verification entry point.
        rows.push(match decode_batch(&graph, &decoder, one) {
            Ok(mut batch) => json!({"shot": index, "correction": batch.shots.remove(0)}),
            Err(error) => json!({"shot": index, "error": error}),
        });
        if (index + 1) % 10 == 0 {
            eprintln!(
                "{} shots in {:.3}s",
                index + 1,
                start.elapsed().as_secs_f64()
            );
        }
    }
    let output = json!({"initialization_seconds": initialization_seconds,
                        "decode_seconds": start.elapsed().as_secs_f64(), "shots": rows});
    fs::write(&args[3], serde_json::to_vec(&output)?)?;
    Ok(())
}
