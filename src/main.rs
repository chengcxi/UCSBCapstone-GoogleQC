use microblossom_toolchain::{Dem, Graph, convert};
use std::{env, fs, path::PathBuf, process, process::Command};

const HELP: &str = "Micro Blossom circuit-to-graph compiler

Usage:
  microblossom-toolchain compile INPUT --output PREFIX [OPTIONS]
  microblossom-toolchain export INPUT.ir.json --output PREFIX [OPTIONS]
  microblossom-toolchain check-correction INPUT.ir.json SHOT.json [--allow-correlated]
  microblossom-toolchain INPUT.ir.json PREFIX [--allow-correlated]  (legacy)

Options:
  --kind circuit|dem       Compile input type (default: circuit)
  --python EXECUTABLE      Python with Stim installed (default: MICROBLOSSOM_PYTHON or python3)
  --windows PLAN.json     Window annotations (compile only; does not cut graphs)
  --weight-scale NUMBER   Positive, finite log-odds weight scale (default: 100)
  --allow-correlated      Explicitly approximate split errors as independent edges
  --micro-blossom BINARY  Run upstream parser and verify graph/edge index preservation

Writes PREFIX.ir.json, PREFIX.graph.json, PREFIX.syndromes.
With --micro-blossom, also writes PREFIX.hardware.json.
Requires Python + Stim only for compile; export is a standalone Rust operation.";

struct Options {
    compile: bool,
    input: PathBuf,
    prefix: PathBuf,
    kind: String,
    python: String,
    windows: Option<PathBuf>,
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
            "--output" | "--kind" | "--python" | "--windows" | "--weight-scale" | "--micro-blossom"
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
    Ok(options)
}

fn extract_ir(options: &Options) -> Result<String, Box<dyn std::error::Error>> {
    if !options.compile {
        return Ok(fs::read_to_string(&options.input)?);
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
    Ok(String::from_utf8(output.stdout)?)
}

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn publish(options: &Options, ir: &str, graph: &Graph) -> Result<(), Box<dyn std::error::Error>> {
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
    let options = parse(args)?;
    let ir = extract_ir(&options)?;
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
    publish(&options, &ir, &graph)
}

fn main() {
    if let Err(error) = run() {
        eprintln!("microblossom-toolchain: {error}");
        process::exit(1);
    }
}
