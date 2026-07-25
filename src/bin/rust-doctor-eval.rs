mod evaluation;

use clap::{Args, Parser, Subcommand};
use evaluation::EvalError;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(
    name = "rust-doctor-eval",
    about = "Corpus, regression, benchmark, and artifact gates for rust-doctor"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate and rank the evidence-backed rule candidate backlog.
    Backlog(BacklogArgs),
    /// Prepare pinned repositories while network access is allowed.
    Prepare(PrepareArgs),
    /// Scan a prepared corpus in credential-free network sandboxes.
    Corpus(CorpusArgs),
    /// Compare candidate corpus NDJSON with an approved baseline.
    Delta(DeltaArgs),
    /// Run fixed performance fixtures and optionally gate a baseline.
    Benchmark(BenchmarkArgs),
    /// Exercise built CLI, MCP, and npm package surfaces.
    Smoke(SmokeArgs),
}

#[derive(Debug, Args)]
struct BacklogArgs {
    #[arg(long, default_value = "evaluation/rule-backlog-v1.json")]
    manifest: PathBuf,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PrepareArgs {
    #[arg(long, default_value = "evaluation/corpus-v1.json")]
    manifest: PathBuf,
    #[arg(long)]
    checkout_root: PathBuf,
    #[arg(long)]
    prepared_out: PathBuf,
    #[arg(long, default_value_t = 900)]
    repository_timeout_secs: u64,
}

#[derive(Debug, Args)]
struct CorpusArgs {
    #[arg(long, default_value = "evaluation/corpus-v1.json")]
    manifest: PathBuf,
    #[arg(long)]
    prepared: PathBuf,
    #[arg(long)]
    checkout_root: PathBuf,
    #[arg(long)]
    binary: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    tool_revision: Option<String>,
    #[arg(long)]
    concurrency: Option<usize>,
    #[arg(long, default_value_t = 600)]
    repository_timeout_secs: u64,
    #[arg(long, default_value_t = 21_600)]
    global_timeout_secs: u64,
    #[arg(long, default_value_t = 16)]
    output_limit_mib: usize,
    #[arg(long, default_value_t = 2_048)]
    sandbox_memory_mib: usize,
    #[arg(long, default_value_t = 64)]
    sandbox_process_limit: usize,
    #[arg(long, default_value_t = 2_048)]
    sandbox_scratch_mib: usize,
}

#[derive(Debug, Args)]
struct DeltaArgs {
    #[arg(long)]
    baseline: PathBuf,
    #[arg(long)]
    baseline_approval: PathBuf,
    #[arg(long)]
    candidate: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    labels: Option<PathBuf>,
    #[arg(long)]
    candidate_approval: Option<PathBuf>,
    /// Write the deterministic promoted-rule review sample before applying label gates.
    #[arg(long)]
    review_template: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct BenchmarkArgs {
    #[arg(long, default_value = "evaluation/benchmarks-v1.json")]
    manifest: PathBuf,
    #[arg(long)]
    binary: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    baseline: Option<PathBuf>,
    #[arg(long, requires = "baseline")]
    baseline_approval: Option<PathBuf>,
    #[arg(long, default_value_t = 3)]
    repetitions: usize,
    /// Measure and emit an unapproved baseline candidate instead of gating.
    #[arg(long, conflicts_with = "baseline")]
    record: bool,
}

#[derive(Debug, Args)]
struct SmokeArgs {
    #[arg(long)]
    binary: PathBuf,
    #[arg(long)]
    no_default_binary: PathBuf,
    #[arg(long, default_value = "schemas/report-v1.schema.json")]
    schema: PathBuf,
    /// Final platform-specific npm tarball produced by the release packer.
    #[arg(long)]
    npm_platform_package: PathBuf,
    /// Final npm wrapper tarball produced by the release packer.
    #[arg(long)]
    npm_wrapper_package: PathBuf,
    #[arg(long, default_value = "bun")]
    bun: PathBuf,
    /// Final native archive to extract and execute (repeat per artifact).
    #[arg(long = "archive", required = true)]
    archives: Vec<PathBuf>,
    /// Verified Cargo package produced by `cargo package --locked`.
    #[arg(long = "crate-package")]
    crate_package: PathBuf,
    #[arg(long, default_value_t = 600)]
    timeout_secs: u64,
}

fn run(cli: Cli) -> Result<(), EvalError> {
    match cli.command {
        Command::Backlog(args) => evaluation::backlog::run(&args.manifest, args.json),
        Command::Prepare(args) => evaluation::corpus::prepare(args),
        Command::Corpus(args) => evaluation::corpus::run(args),
        Command::Delta(args) => evaluation::delta::run(args),
        Command::Benchmark(args) => evaluation::benchmark::run(args),
        Command::Smoke(args) => evaluation::smoke::run(args),
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(error.exit_code())
        }
    }
}
