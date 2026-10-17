//! The `opencodifier-itemgen` CLI: generates the solver-verified
//! relational/contrastive corpus slice (E1-F1, decision record D37).
//!
//! Writes three artifacts: the train JSONL, the held-out probe JSONL,
//! and a manifest JSON recording every count, census, and guard
//! outcome the downstream consumers need to trust the corpus. Exit
//! status is nonzero when generation cannot guarantee its own output
//! (an item could not be verified within the retry budget, a record
//! could not be serialized, or a guard's precondition failed) — the
//! corpus never ships with a silently degraded guard.

use std::error::Error;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use clap::Parser;
use opencodifier_itemgen::{GenConfig, SuiteGuard, generate, manifest_value};

/// Generate solver-verified relational/contrastive decision items.
#[derive(Debug, Parser)]
#[command(name = "opencodifier-itemgen", version)]
struct Args {
    /// Number of items to generate (train + probe).
    #[arg(long, default_value_t = 2000)]
    n: usize,
    /// Run seed: identical seed and n reproduce byte-identical files.
    #[arg(long, default_value_t = 0x0C0D_1F0_2026)]
    seed: u64,
    /// Every k-th item (0-based) goes to the probe file. 0 keeps all.
    #[arg(long, default_value_t = 20)]
    probe_every: usize,
    /// Family rotation pool as a comma list of codes
    /// (rcc,rca,hb,frc,sb). Empty = all five families.
    #[arg(long, default_value = "")]
    families: String,
    /// In root-cause slots, every k-th item is emitted as a minimal
    /// pair (base + one-fact-corrupted twin). 0 disables pairs.
    #[arg(long, default_value_t = 0)]
    pair_every: usize,
    /// Train output JSONL path.
    #[arg(long)]
    out_train: PathBuf,
    /// Probe output JSONL path.
    #[arg(long)]
    out_probe: PathBuf,
    /// Suite JSON used as the near-duplicate collision guard.
    #[arg(long)]
    suite: PathBuf,
    /// Manifest output path.
    #[arg(long)]
    manifest: PathBuf,
}

/// Opens an output file for writing, with the path in any error.
fn create_output(path: &Path) -> Result<std::fs::File, Box<dyn Error>> {
    std::fs::File::create(path)
        .map_err(|error| format!("create {}: {error}", path.display()).into())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let mut families = Vec::new();
    for code in args.families.split(',') {
        let code = code.trim();
        if code.is_empty() {
            continue;
        }
        let family = opencodifier_itemgen::Family::from_code(code)
            .ok_or_else(|| format!("unknown family code {code:?} (rcc,rca,hb,frc,sb)"))?;
        families.push(family);
    }
    let config = GenConfig {
        n: args.n,
        seed: args.seed,
        probe_every: args.probe_every,
        families,
        pair_every: args.pair_every,
    };

    let suite = SuiteGuard::load(&args.suite).map_err(|error| format!("suite guard: {error}"))?;

    let mut train = std::io::BufWriter::new(create_output(&args.out_train)?);
    let mut probe = std::io::BufWriter::new(create_output(&args.out_probe)?);
    // The emit callback cannot propagate errors through its `()`
    // contract, so the first write failure is captured here and
    // surfaced once generation returns.
    let mut write_error: Option<std::io::Error> = None;

    let result = generate(&config, Some(&suite), |is_probe, record, _| {
        if write_error.is_some() {
            return;
        }
        let writer: &mut dyn std::io::Write = if is_probe { &mut probe } else { &mut train };
        let written: Result<(), std::io::Error> = serde_json::to_writer(&mut *writer, &record)
            .map_err(std::io::Error::other)
            .and_then(|()| writeln!(writer));
        if let Err(error) = written {
            write_error = Some(error);
        }
    });

    let stats = match result {
        Ok(stats) => stats,
        Err(opencodifier_itemgen::GenError::Unverifiable { family, index, last_error }) => {
            return Err(format!(
                "item {index} ({family:?}) exhausted its verification retries: {last_error}"
            )
            .into());
        }
    };
    if let Some(error) = write_error {
        return Err(format!("write record: {error}").into());
    }

    train.flush().map_err(|error| format!("flush train: {error}"))?;
    probe.flush().map_err(|error| format!("flush probe: {error}"))?;

    let manifest = manifest_value(&config, &stats);
    let manifest_text = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("serialize manifest: {error}"))?;
    std::fs::write(&args.manifest, manifest_text)
        .map_err(|error| format!("write {}: {error}", args.manifest.display()))?;

    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "generated {} train + {} probe (seed {}, probe_every {})",
        stats.train, stats.probe, config.seed, config.probe_every
    )?;
    writeln!(
        out,
        "rejected: {} verification, {} suite collisions",
        stats.rejected, stats.collisions
    )?;
    if stats.pairs > 0 || stats.pairs_rejected > 0 {
        writeln!(
            out,
            "minimal pairs: {} emitted, {} twins rejected",
            stats.pairs, stats.pairs_rejected
        )?;
    }
    if !stats.score_levels.is_empty() {
        let levels = serde_json::to_string(&stats.score_levels)
            .map_err(|error| format!("serialize level census: {error}"))?;
        writeln!(out, "score levels: {levels}")?;
    }
    if let Some(lexical) = manifest["lexical_probe"].as_object() {
        for (family, rate) in lexical {
            writeln!(
                out,
                "lexical {family}: solved {} / {} ({})",
                rate["solved"].as_u64().unwrap_or(0),
                rate["total"].as_u64().unwrap_or(0),
                rate["rate"],
            )?;
        }
    }
    Ok(())
}
