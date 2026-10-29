use microblossom_toolchain::benchmark::compare_cpu_decoders;
use microblossom_toolchain::decoding::{CorrectionBatch, ShotBatch, decode_batch, prepare_decoder};
use microblossom_toolchain::window::{WindowGraph, build_window_graphs};
use microblossom_toolchain::{Dem, Graph, convert};
use std::{env, fs, path::PathBuf, process, process::Command};

const HELP: &str = "Micro Blossom circuit-to-graph compiler

Usage:
  microblossom-toolchain compile INPUT --output PREFIX [OPTIONS]
  microblossom-toolchain export INPUT.ir.json --output PREFIX [OPTIONS]
  microblossom-toolchain check-correction INPUT.ir.json SHOT.json [--allow-correlated]
  microblossom-toolchain benchmark INPUT.ir.json --shots SHOTS.json --output REPORT.json
      [--warmup PASSES] [--repeats PASSES] [--weight-scale NUMBER] [--allow-correlated]
  microblossom-toolchain INPUT.ir.json PREFIX [--allow-correlated]  (legacy)

Options:
  --kind circuit|dem       Compile input type (default: circuit)
  --python EXECUTABLE      Python with Stim installed (default: MICROBLOSSOM_PYTHON or python3)
  --windows PLAN.json     Annotate and cut window graphs (compile only)
  --shots INPUT.json      Decode supplied detector shots (requires annotated windows)
  --sample-shots COUNT    Sample Stim and decode COUNT shots (compile only, requires --windows)
  --seed NUMBER           Reproducible Stim sampling seed (with --sample-shots; default: 0)
  --weight-scale NUMBER   Positive, finite log-odds weight scale (default: 100)
  --allow-correlated      Explicitly approximate split errors as independent edges
  --micro-blossom BINARY  Run upstream parser and verify graph/edge index preservation

Writes PREFIX.ir.json, PREFIX.graph.json, PREFIX.syndromes.
With annotated windows writes PREFIX.windows.json; with shot input also
writes PREFIX.corrections.json. With --micro-blossom writes PREFIX.hardware.json.
Benchmark compares the global Micro Blossom CPU solver with the sequential
CPU window decoder on identical shots. Timings do not include Stim or FPGA.
Benchmark defaults: 2 warmup passes, 20 measured passes; --shots is required.
Requires Python + Stim only for compile; export, benchmark and supplied shots are Rust-only.";

struct Options {
    compile: bool,
    input: PathBuf,
    prefix: PathBuf,
    kind: String,
    python: String,
    windows: Option<PathBuf>,
    shots: Option<PathBuf>,
    sample_shots: Option<usize>,
    seed: Option<u64>,
    scale: f64,
    correlated: bool,
    upstream: Option<String>,
}

fn parse(args: Vec<String>) -> Result<Options, String> {
    if args.is_empty() {
        return Err(HELP.into());
    }
    let modern = matches!(args[0].as_str(), "compile" | "export");
    let mut options = Options {
        compile: args[0] == "compile",
        input: PathBuf::new(),
        prefix: PathBuf::new(),
        kind: "circuit".into(),
        python: env::var("MICROBLOSSOM_PYTHON").unwrap_or_else(|_| "python3".into()),
        windows: None,
        shots: None,
        sample_shots: None,
        seed: None,
        scale: 100.0,
        correlated: false,
        upstream: None,
    };
    if !modern {
        if !(args.len() == 2 || (args.len() == 3 && args[2] == "--allow-correlated")) {
            return Err(HELP.into());
        }
        options.input = args[0].clone().into();
        options.prefix = args[1].clone().into();
        options.correlated = args.len() == 3;
        if options.input.as_os_str().is_empty() || options.prefix.file_name().is_none() {
            return Err("legacy invocation requires nonempty input and output file prefix".into());
        }
        return Ok(options);
    }
    if args.len() < 2 || args[1].starts_with('-') {
        return Err("missing input path".into());
    }
    options.input = args[1].clone().into();
    let mut i = 2;
    let mut seen = std::collections::BTreeSet::new();
    while i < args.len() {
        let flag = args[i].as_str();
        if !seen.insert(flag) {
            return Err(format!("duplicate option: {flag}"));
        }
        if flag == "--allow-correlated" {
            options.correlated = true;
            i += 1;
            continue;
        }
        if !matches!(
            flag,
            "--output"
                | "--kind"
                | "--python"
                | "--windows"
                | "--shots"
                | "--sample-shots"
                | "--seed"
                | "--weight-scale"
                | "--micro-blossom"
        ) {
            return Err(format!("unknown option: {flag}"));
        }
        let value = args
            .get(i + 1)
            .filter(|v| !v.is_empty() && !v.starts_with("--"))
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag {
            "--output" => options.prefix = value.into(),
            "--kind" | "--python" | "--windows" if !options.compile => {
                return Err(format!("{flag} is only valid for compile"));
            }
            "--kind" => options.kind = value.clone(),
            "--python" => options.python = value.clone(),
            "--windows" => options.windows = Some(value.into()),
            "--shots" => options.shots = Some(value.into()),
            "--sample-shots" if !options.compile => {
                return Err("--sample-shots is only valid for compile".into());
            }
            "--sample-shots" => {
                let count: usize = value.parse().map_err(|_| "invalid --sample-shots count")?;
                if !(1..=100_000).contains(&count) {
                    return Err("--sample-shots must be between 1 and 100000".into());
                }
                options.sample_shots = Some(count);
            }
            "--seed" => {
                options.seed = Some(value.parse().map_err(|_| "invalid --seed")?);
            }
            "--weight-scale" => {
                options.scale = value.parse().map_err(|_| "invalid weight scale")?
            }
            "--micro-blossom" => options.upstream = Some(value.clone()),
            _ => unreachable!(),
        }
        i += 2;
    }
    if options.prefix.as_os_str().is_empty() || options.prefix.file_name().is_none() {
        return Err("--output requires a file prefix".into());
    }
    if !matches!(options.kind.as_str(), "circuit" | "dem") {
        return Err("--kind must be circuit or dem".into());
    }
    if !options.scale.is_finite() || options.scale <= 0.0 {
        return Err("weight scale must be positive and finite".into());
    }
    if options.shots.is_some() && options.sample_shots.is_some() {
        return Err("--shots and --sample-shots are mutually exclusive".into());
    }
    if options.sample_shots.is_some() && options.windows.is_none() {
        return Err("--sample-shots requires --windows".into());
    }
    if options.seed.is_some() && options.sample_shots.is_none() {
        return Err("--seed requires --sample-shots".into());
    }
    Ok(options)
}

#[derive(serde::Deserialize)]
struct SampledBridgeOutput {
    ir: serde_json::Value,
    shots: ShotBatch,
}

fn extract_ir(
    options: &Options,
) -> Result<(String, Option<ShotBatch>), Box<dyn std::error::Error>> {
    if !options.compile {
        return Ok((fs::read_to_string(&options.input)?, None));
    }
    // Embed the bridge so installed binaries do not depend on a checkout path.
    let mut command = Command::new(&options.python);
    command
        .args(["-c", include_str!("../tools/stim_export.py")])
        .arg(&options.input)
        .args(["--kind", &options.kind, "--output", "-"]);
    if let Some(windows) = &options.windows {
        command.arg("--windows").arg(windows);
    }
    if let Some(count) = options.sample_shots {
        command.arg("--sample-shots").arg(count.to_string());
        command
            .arg("--seed")
            .arg(options.seed.unwrap_or(0).to_string());
    }
    let output = command.output().map_err(|e| {
        format!(
            "could not start Python '{}': {e}; use --python or MICROBLOSSOM_PYTHON",
            options.python
        )
    })?;
    if !output.status.success() {
        return Err(format!(
            "Stim bridge failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    let stdout = String::from_utf8(output.stdout)?;
    if options.sample_shots.is_some() {
        let sampled: SampledBridgeOutput = serde_json::from_str(&stdout)
            .map_err(|e| format!("invalid sampled Stim bridge output: {e}"))?;
        Ok((
            serde_json::to_string_pretty(&sampled.ir)? + "\n",
            Some(sampled.shots),
        ))
    } else {
        Ok((stdout, None))
    }
}

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn publish(
    options: &Options,
    ir: &str,
    graph: &Graph,
    windows: Option<&[WindowGraph]>,
    corrections: Option<&CorrectionBatch>,
) -> Result<(), Box<dyn std::error::Error>> {
    let parent = options
        .prefix
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    fs::create_dir_all(parent)?;
    let mut sequence = 0;
    let staging = loop {
        let path = parent.join(format!(".microblossom-stage-{}-{sequence}", process::id()));
        match fs::create_dir(&path) {
            Ok(()) => break Staging(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => sequence += 1,
            Err(e) => return Err(e.into()),
        }
    };
    let mut files = vec!["ir.json", "graph.json", "syndromes"];
    fs::write(staging.0.join("ir.json"), ir)?;
    fs::write(
        staging.0.join("graph.json"),
        serde_json::to_string_pretty(graph)? + "\n",
    )?;
    fs::write(staging.0.join("syndromes"), graph.parser_input()?)?;
    if let Some(windows) = windows {
        let bundle = serde_json::json!({"schema_version": 1, "windows": windows});
        fs::write(
            staging.0.join("windows.json"),
            serde_json::to_string_pretty(&bundle)? + "\n",
        )?;
        files.push("windows.json");
    }
    if let Some(corrections) = corrections {
        fs::write(
            staging.0.join("corrections.json"),
            serde_json::to_string_pretty(corrections)? + "\n",
        )?;
        files.push("corrections.json");
    }
    if let Some(binary) = &options.upstream {
        let output = Command::new(binary)
            .arg("parser")
            .arg(staging.0.join("syndromes"))
            .arg("--graph-file")
            .arg(staging.0.join("hardware.json"))
            .output()
            .map_err(|e| format!("could not start Micro Blossom '{binary}': {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "Micro Blossom parser failed ({}): {}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim(),
                String::from_utf8_lossy(&output.stdout).trim()
            )
            .into());
        }
        let hardware: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(staging.0.join("hardware.json"))?)?;
        graph.verify_hardware_graph(&hardware)?;
        files.push("hardware.json");
    }
    let destinations: Vec<_> = files
        .iter()
        .map(|suffix| {
            let mut path = options.prefix.as_os_str().to_os_string();
            path.push(format!(".{suffix}"));
            PathBuf::from(path)
        })
        .collect();
    for path in &destinations {
        if path.is_dir() {
            return Err(format!("output path is a directory: {}", path.display()).into());
        }
    }
    // Conversion and upstream validation finish before touching existing outputs.
    // Each rename is atomic; this is not a multi-file filesystem transaction.
    for (suffix, destination) in files.iter().zip(&destinations) {
        fs::rename(staging.0.join(suffix), destination)?;
        println!("{}", destination.display());
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CorrectionInput {
    defects: Vec<usize>,
    edges: Vec<usize>,
}

fn check_correction(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if !(args.len() == 2 || (args.len() == 3 && args[2] == "--allow-correlated")) {
        return Err("usage: check-correction INPUT.ir.json SHOT.json [--allow-correlated]".into());
    }
    let dem: Dem = serde_json::from_str(&fs::read_to_string(&args[0])?)?;
    let shot: CorrectionInput = serde_json::from_str(&fs::read_to_string(&args[1])?)?;
    let graph = convert(dem, 100.0, args.len() == 3)?;
    let parity = graph.evaluate_correction(&shot.edges, &shot.defects)?;
    println!("{}", serde_json::to_string_pretty(&parity)?);
    if !parity.residual_syndrome.is_empty() {
        return Err("correction does not reproduce the supplied syndrome".into());
    }
    Ok(())
}

fn run_benchmark(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.is_empty() || args[0].starts_with('-') {
        return Err("usage: benchmark INPUT.ir.json --shots SHOTS.json --output REPORT.json [--warmup N] [--repeats N]".into());
    }
    let input = PathBuf::from(&args[0]);
    let mut shots: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut warmup = 2;
    let mut repeats = 20;
    let mut scale = 100.0;
    let mut correlated = false;
    let mut seen = std::collections::BTreeSet::new();
    let mut i = 1;
    while i < args.len() {
        let flag = args[i].as_str();
        if !seen.insert(flag) {
            return Err(format!("duplicate benchmark option: {flag}").into());
        }
        if flag == "--allow-correlated" {
            correlated = true;
            i += 1;
            continue;
        }
        if !matches!(
            flag,
            "--shots" | "--output" | "--warmup" | "--repeats" | "--weight-scale"
        ) {
            return Err(format!("unknown benchmark option: {flag}").into());
        }
        let value = args
            .get(i + 1)
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag {
            "--shots" => shots = Some(value.into()),
            "--output" => output = Some(value.into()),
            "--warmup" => {
                warmup = value
                    .parse::<usize>()
                    .map_err(|_| "invalid --warmup passes")?
            }
            "--repeats" => {
                repeats = value
                    .parse::<usize>()
                    .map_err(|_| "invalid --repeats passes")?
            }
            "--weight-scale" => scale = value.parse::<f64>().map_err(|_| "invalid weight scale")?,
            _ => unreachable!(),
        }
        i += 2;
    }
    let shots = shots.ok_or("benchmark requires --shots")?;
    let output = output.ok_or("benchmark requires --output")?;
    if output.file_name().is_none() {
        return Err("--output requires a report filename".into());
    }
    if !scale.is_finite() || scale <= 0.0 {
        return Err("weight scale must be positive and finite".into());
    }
    let dem: Dem = serde_json::from_str(&fs::read_to_string(input)?)?;
    let graph = convert(dem, scale, correlated)?;
    let cut = build_window_graphs(&graph)?;
    let decoder = prepare_decoder(&graph, &cut)?;
    let batch: ShotBatch = serde_json::from_str(&fs::read_to_string(shots)?)?;
    let report = compare_cpu_decoders(&graph, &decoder, &batch, warmup, repeats)?;
    // Never publish a partial report, nor replace an earlier report if an
    // input, decoder or parity check fails.
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    fs::create_dir_all(parent)?;
    let mut sequence = 0;
    let staging = loop {
        let path = parent.join(format!(
            ".microblossom-benchmark-{}-{sequence}",
            process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => break Staging(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => sequence += 1,
            Err(e) => return Err(e.into()),
        }
    };
    fs::write(
        staging.0.join("report.json"),
        serde_json::to_string_pretty(&report)? + "\n",
    )?;
    if output.is_dir() {
        return Err(format!("output path is a directory: {}", output.display()).into());
    }
    fs::rename(staging.0.join("report.json"), &output)?;
    println!("{}", output.display());
    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = env::args_os()
        .skip(1)
        .map(|s| {
            s.into_string()
                .map_err(|_| "command-line arguments must be UTF-8")
        })
        .collect::<Result<Vec<_>, _>>()?;
    if args.iter().any(|s| matches!(s.as_str(), "--help" | "-h")) {
        println!("{HELP}");
        return Ok(());
    }
    if args == ["--version"] {
        println!("microblossom-toolchain {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.first().is_some_and(|s| s == "check-correction") {
        return check_correction(&args[1..]);
    }
    if args.first().is_some_and(|s| s == "benchmark") {
        return run_benchmark(&args[1..]);
    }
    let options = parse(args)?;
    let (ir, sampled_shots) = extract_ir(&options)?;
    let dem: Dem = serde_json::from_str(&ir).map_err(|e| format!("invalid DEM IR: {e}"))?;
    let graph = convert(dem, options.scale, options.correlated)?;
    if graph.correlated_components_approximated {
        eprintln!("warning: correlated DEM components are approximated as independent edges");
    }
    let zero_weights = graph
        .initializer
        .weighted_edges
        .iter()
        .filter(|e| e.2 == 0)
        .count();
    if zero_weights > 0 {
        eprintln!(
            "warning: {zero_weights} edge(s) have zero weight (p=0.5 or quantization); verify backend support"
        );
    }
    // Do all window cutting and decoding before touching existing outputs.
    let windows = if graph.window_annotations.is_null() {
        None
    } else {
        Some(build_window_graphs(&graph)?)
    };
    let shots = match options.shots.as_ref() {
        Some(path) => Some(
            serde_json::from_str::<ShotBatch>(&fs::read_to_string(path)?)
                .map_err(|e| format!("invalid shots input: {e}"))?,
        ),
        None => sampled_shots,
    };
    let corrections = if let Some(shots) = shots {
        let cut = windows
            .as_deref()
            .ok_or("--shots requires annotated windows")?;
        let decoder = prepare_decoder(&graph, cut)?;
        Some(decode_batch(&graph, &decoder, shots)?)
    } else {
        None
    };
    publish(
        &options,
        &ir,
        &graph,
        windows.as_deref(),
        corrections.as_ref(),
    )
}

fn main() {
    if let Err(error) = run() {
        eprintln!("microblossom-toolchain: {error}");
        process::exit(1);
    }
}
