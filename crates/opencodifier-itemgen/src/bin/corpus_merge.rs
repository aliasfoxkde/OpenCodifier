//! The `opencodifier-corpus-merge` CLI: assembles the `e1-sft-v2`
//! training rows from the prepped merged-v3 base plus the full-weight
//! generated slices (TRAINING.md §9.7 B1/B2, §9.8 adopted knobs).
//!
//! The inputs are PREPPED row files (the `decision_sft_prep` output
//! shape: `qtype`, `label`, `source`, `segments`, ...), so the merge
//! operates on exactly what the trainer consumes and never has to
//! re-render anything. The base is rebalanced by temperature sampling
//! over question families (p ∝ N^(1/T), T = 2.5), with the dominant
//! family capped at 40% of the final mix, per-family oversampling
//! capped at 4×, and the degenerate score class deprioritized so
//! non-constant score rows hold ≥ 35% of the score slice. Generated
//! slices enter at full weight. Everything is seeded and byte-
//! reproducible; the manifest records every census, and a violated
//! gate fails the run AFTER the manifest is written so the evidence
//! always survives. The gates are shape-aware: both presume the
//! generated slices exist (the 40% cap needs non-base rows to dilute
//! the dominant family; the 35% non-constant floor is unreachable
//! from the base, whose score bucket is structurally modal-dominant),
//! so a slice-less rebalance run records them as not-applicable with
//! the reason, and reports the raw mix shares unconditionally.

use std::collections::BTreeMap;
use std::error::Error;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use clap::Parser;
use opencodifier_itemgen::rng::Rng;
use serde_json::Value;

/// Temperature for family-share sampling: `share_f ∝ N_f^(1/T)`. `T =
/// 2.5` flattens a 9:1 count ratio into roughly 1.9:1 weight — strong
/// rebalancing without erasing the base distribution (§9.8).
const TEMPERATURE_T: f64 = 2.5;

/// Cap on any single question family's share of the FINAL mix (§9.7
/// B1: the dominant `noul` family must not own the corpus).
const FAMILY_SHARE_CAP: f64 = 0.40;

/// Cap on how many copies of a family's rows the sampler may take,
/// relative to the family's own size (§9.8 adopted knob).
const OVERSAMPLE_CAP: usize = 4;

/// Families whose rows are NEVER oversampled: duplicating a bucket
/// whose mass sits on one label amplifies exactly the degeneracy the
/// merge exists to fix (§9.7 B2), so the score bucket enters at most
/// 1× and the freed budget flows to the other families.
const NEVER_OVERSAMPLE: &[&str] = &["score"];

/// The score slice must hold at least this share of non-constant rows
/// after the merge (§9.7 B2; measured: the base is 89.1% constant).
const SCORE_NONCONSTANT_MIN: f64 = 0.35;

/// Assemble the e1-sft-v2 rows from the prepped base and slices.
#[derive(Debug, Parser)]
#[command(name = "opencodifier-corpus-merge", version)]
struct Args {
    /// Prepped base rows (merged-v3 through `decision_sft_prep`).
    #[arg(long)]
    base: PathBuf,
    /// Prepped full-weight slice rows; repeatable.
    #[arg(long = "slice")]
    slices: Vec<PathBuf>,
    /// Output rows JSONL.
    #[arg(long)]
    out: PathBuf,
    /// Manifest output path.
    #[arg(long)]
    manifest: PathBuf,
    /// Target total row count (base sample + slices).
    #[arg(long, default_value_t = 178_000)]
    target_total: usize,
    /// Run seed for every sampling decision.
    #[arg(long, default_value_t = 0x0C0D_1F0_2026)]
    seed: u64,
}

/// One prepped row: the original line plus the label the merge
/// decisions read, parsed once at load.
struct Row {
    line: String,
    label: Option<String>,
}

/// A loaded rows file, indexed by question family.
struct Bucket {
    rows: Vec<Row>,
    qtypes: BTreeMap<String, Vec<usize>>,
}

impl Bucket {
    fn new() -> Self {
        Self { rows: Vec::new(), qtypes: BTreeMap::new() }
    }

    fn count(&self, qtype: &str) -> usize {
        self.qtypes.get(qtype).map_or(0, Vec::len)
    }

    fn total(&self) -> usize {
        self.rows.len()
    }

    fn family_counts(&self) -> BTreeMap<String, usize> {
        self.qtypes.iter().map(|(family, rows)| (family.clone(), rows.len())).collect()
    }
}

/// Reads a prepped rows file into the bucket, recording each row's
/// question family and label.
fn load_rows(path: &Path, bucket: &mut Bucket) -> Result<(), Box<dyn Error>> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: Value = serde_json::from_str(line)
            .map_err(|error| format!("{}:{}: {error}", path.display(), index + 1))?;
        let qtype = row
            .get("qtype")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{}:{}: row without qtype", path.display(), index + 1))?
            .to_owned();
        let label = row.get("label").and_then(Value::as_str).map(str::to_owned);
        bucket.qtypes.entry(qtype).or_default().push(bucket.rows.len());
        bucket.rows.push(Row { line: line.to_owned(), label });
    }
    Ok(())
}

/// Temperature shares over the base family counts, with the 40% cap
/// applied and the clamped excess redistributed to the uncapped
/// families until no family exceeds the cap.
fn temperature_shares(counts: &BTreeMap<String, usize>) -> BTreeMap<String, f64> {
    let weights: BTreeMap<String, f64> = counts
        .iter()
        .map(|(family, count)| {
            #[allow(clippy::cast_precision_loss)] // corpus census counts
            let weight = (*count as f64).powf(1.0 / TEMPERATURE_T);
            (family.clone(), weight)
        })
        .collect();
    let total_weight: f64 = weights.values().sum();
    let mut shares: BTreeMap<String, f64> =
        weights.iter().map(|(family, w)| (family.clone(), w / total_weight)).collect();
    for _ in 0..8 {
        let excess: f64 = shares
            .values()
            .filter(|share| **share > FAMILY_SHARE_CAP)
            .map(|share| share - FAMILY_SHARE_CAP)
            .sum();
        if excess <= f64::EPSILON {
            break;
        }
        for share in shares.values_mut() {
            if *share > FAMILY_SHARE_CAP {
                *share = FAMILY_SHARE_CAP;
            }
        }
        let free: f64 = shares.values().filter(|share| **share < FAMILY_SHARE_CAP).sum();
        if free <= f64::EPSILON {
            break;
        }
        for share in shares.values_mut() {
            if *share < FAMILY_SHARE_CAP {
                *share += excess * (*share / free);
            }
        }
    }
    shares
}

/// The per-family take plan: temperature shares of the base budget,
/// each clamped to the oversample cap, with clamped excess
/// redistributed proportionally until the plan settles under the cap.
#[allow(clippy::cast_precision_loss)] // budget is a corpus count
#[allow(clippy::cast_sign_loss)] // shares are in [0, 1] by construction
#[allow(clippy::cast_possible_truncation)] // share × budget, then rounded
fn take_plan(
    base_counts: &BTreeMap<String, usize>,
    shares: &BTreeMap<String, f64>,
    budget: usize,
) -> BTreeMap<String, usize> {
    let mut take: BTreeMap<String, usize> = shares
        .iter()
        .map(|(family, share)| (family.clone(), (share * budget as f64).round() as usize))
        .collect();
    let family_available = |family: &str| {
        let count = base_counts.get(family).copied().unwrap_or(0);
        if NEVER_OVERSAMPLE.contains(&family) { count } else { count * OVERSAMPLE_CAP }
    };
    loop {
        let mut excess = 0usize;
        for (family, count) in &mut take {
            let available = family_available(family);
            if *count > available {
                excess += *count - available;
                *count = available;
            }
        }
        if excess == 0 {
            break;
        }
        let free: usize =
            take.iter().map(|(family, count)| family_available(family) - *count).sum();
        if free == 0 {
            break;
        }
        let give = excess.min(free);
        for (family, count) in &mut take {
            let room = family_available(family) - *count;
            *count += room.min(give * room / free.max(1));
        }
    }
    take
}

/// The modal (degenerate) score label of the base corpus — measured,
/// not assumed: merged-v3's score gold is "0" for 89.1% of rows.
fn modal_score_label(bucket: &Bucket) -> Option<String> {
    let mut labels: BTreeMap<String, usize> = BTreeMap::new();
    for index in bucket.qtypes.get("score")? {
        if let Some(label) = &bucket.rows[*index].label {
            *labels.entry(label.clone()).or_default() += 1;
        }
    }
    labels.into_iter().max_by_key(|(_, count)| *count).map(|(label, _)| label)
}

/// The drawn base sample plus every census the manifest reports.
struct BaseSelection {
    rows: Vec<String>,
    census: BTreeMap<String, usize>,
    oversample_factors: BTreeMap<String, String>,
    score_degenerate_taken: usize,
    score_nonconstant_taken: usize,
}

/// Draws the base sample family by family per the take plan. Score
/// draws its non-constant rows before the degenerate modal class, so a
/// modest take already moves the non-constant share; choice families
/// draw plain seeded shuffle order, repeating cyclically on oversample
/// passes.
fn select_base(
    base: &Bucket,
    take: &BTreeMap<String, usize>,
    degenerate_label: Option<&str>,
    rng: &mut Rng,
) -> BaseSelection {
    let mut selection = BaseSelection {
        rows: Vec::new(),
        census: BTreeMap::new(),
        oversample_factors: BTreeMap::new(),
        score_degenerate_taken: 0,
        score_nonconstant_taken: 0,
    };
    let mut permutation: Vec<usize> = Vec::new();
    for (family, rows) in &base.qtypes {
        let want = take.get(family).copied().unwrap_or(0);
        permutation.clear();
        permutation.extend_from_slice(rows);
        rng.shuffle(&mut permutation);
        let order: Vec<usize> = if family == "score" {
            let mut front = Vec::new();
            let mut back = Vec::new();
            for &index in &permutation {
                if base.rows[index].label.as_deref() == degenerate_label {
                    back.push(index);
                } else {
                    front.push(index);
                }
            }
            front.extend(back);
            front
        } else {
            permutation.clone()
        };
        let passes = want.div_ceil(rows.len().max(1));
        if passes > 1 {
            selection.oversample_factors.insert(family.clone(), format!("{passes}×"));
        }
        for slot in 0..want {
            let index = order[slot % order.len()];
            selection.rows.push(base.rows[index].line.clone());
            *selection.census.entry(family.clone()).or_default() += 1;
            if family == "score" {
                if base.rows[index].label.as_deref() == degenerate_label {
                    selection.score_degenerate_taken += 1;
                } else {
                    selection.score_nonconstant_taken += 1;
                }
            }
        }
    }
    selection
}

/// Writes the final mix in a seeded shuffle order; returns the digest
/// of the file it wrote.
fn write_rows(
    out: &Path,
    slices: &[String],
    sampled: &[String],
    rng: &mut Rng,
) -> Result<String, Box<dyn Error>> {
    use sha2::Digest as _;
    use std::io::Read as _;

    let mut final_rows: Vec<String> = Vec::with_capacity(slices.len() + sampled.len());
    final_rows.extend(slices.iter().cloned());
    final_rows.extend(sampled.iter().cloned());
    let mut order: Vec<usize> = (0..final_rows.len()).collect();
    rng.shuffle(&mut order);

    let file =
        std::fs::File::create(out).map_err(|error| format!("create {}: {error}", out.display()))?;
    let mut writer = std::io::BufWriter::new(file);
    for index in order {
        writeln!(writer, "{}", final_rows[index])
            .map_err(|error| format!("write rows: {error}"))?;
    }
    writer.flush().map_err(|error| format!("flush rows: {error}"))?;

    let mut file =
        std::fs::File::open(out).map_err(|error| format!("open {}: {error}", out.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// The §9.7 B6 gate report over the final mix: the gates that were
/// evaluated (a fail aborts the run after the manifest is written),
/// the gates whose preconditions this corpus shape does not meet, and
/// the mix shares that back both lists.
struct GateReport {
    /// Evaluated gates (name → pass/fail).
    evaluated: BTreeMap<String, bool>,
    /// Skipped gates (name → why the precondition is absent).
    not_applicable: BTreeMap<String, String>,
    /// Final-mix `noul` share, reported even when the gate is skipped.
    noul_share: f64,
    /// Share of the score rows that are non-constant (the gate's own
    /// denominator), reported even when the gate is skipped.
    score_nonconstant_share: f64,
}

/// The §9.7 B6 gates over the final mix, applied only where their
/// preconditions hold. Both gates presume the generated slices exist:
/// the 40% dominant-family cap needs non-base rows to dilute `noul`
/// (the base universe alone cannot fill a non-noul budget at the
/// target size), and the 35% non-constant score floor presumes the
/// generated score slice (the base score bucket is structurally
/// modal-dominant). A gate whose precondition is absent is recorded
/// under `not_applicable` instead of failing the run — never silently.
fn evaluate_gates(
    has_slices: bool,
    slice_score_rows: usize,
    final_census: &BTreeMap<String, usize>,
    out_rows: usize,
    score_nonconstant: usize,
) -> GateReport {
    #[allow(clippy::cast_precision_loss)] // corpus census counts
    let share = |count: usize| count as f64 / out_rows.max(1) as f64;
    let noul_share = share(final_census.get("noul").copied().unwrap_or(0));
    let score_total = final_census.get("score").copied().unwrap_or(0);
    // The score share's denominator is the score slice itself (§9.7
    // B2: "non-constant rows hold ≥ 35% of the score slice"), not the
    // whole mix.
    #[allow(clippy::cast_precision_loss)] // corpus census counts
    let score_nonconstant_share =
        if score_total > 0 { score_nonconstant as f64 / score_total as f64 } else { 0.0 };

    let mut evaluated = BTreeMap::new();
    let mut not_applicable = BTreeMap::new();
    if has_slices {
        evaluated.insert("noul_share_le_cap".to_owned(), noul_share <= FAMILY_SHARE_CAP + 0.005);
    } else {
        not_applicable.insert(
            "noul_share_le_cap".to_owned(),
            "slice-less run: the 40% final-mix cap presumes generated slices \
             dilute the dominant family; the base sample alone cannot satisfy it"
                .to_owned(),
        );
    }
    if slice_score_rows > 0 {
        evaluated.insert(
            "score_nonconstant_ge_min".to_owned(),
            score_nonconstant_share >= SCORE_NONCONSTANT_MIN,
        );
    } else {
        not_applicable.insert(
            "score_nonconstant_ge_min".to_owned(),
            "no score-family slice: the 35% non-constant floor presumes the \
             generated score slice; the base score bucket is structurally \
             modal-dominant"
                .to_owned(),
        );
    }
    GateReport { evaluated, not_applicable, noul_share, score_nonconstant_share }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    let mut base = Bucket::new();
    load_rows(&args.base, &mut base)?;
    let mut additions = Bucket::new();
    for slice in &args.slices {
        load_rows(slice, &mut additions)?;
    }

    let base_counts = base.family_counts();
    let additions_counts = additions.family_counts();
    if base.total() == 0 {
        return Err(format!("base {} has no rows", args.base.display()).into());
    }
    let fixed_total = additions.total();
    if fixed_total >= args.target_total {
        return Err(format!(
            "slices ({fixed_total} rows) already meet the target ({})",
            args.target_total
        )
        .into());
    }
    let budget = args.target_total - fixed_total;

    let degenerate_label = modal_score_label(&base);
    let shares = temperature_shares(&base_counts);
    let take = take_plan(&base_counts, &shares, budget);
    let mut rng = Rng::new(args.seed);
    let selection = select_base(&base, &take, degenerate_label.as_deref(), &mut rng);
    let slice_lines: Vec<String> = additions.rows.iter().map(|row| row.line.clone()).collect();
    let rows_sha256 = write_rows(&args.out, &slice_lines, &selection.rows, &mut rng)?;
    let out_rows = fixed_total + selection.rows.len();

    // Final census: slices counted by their own labels (never assumed
    // non-constant), plus the sampled base censuses.
    let mut final_census: BTreeMap<String, usize> = additions_counts.clone();
    for (family, count) in &selection.census {
        *final_census.entry(family.clone()).or_default() += count;
    }
    let slice_score_nonconstant = additions.qtypes.get("score").map_or(0, |rows| {
        rows.iter()
            .filter(|&&index| additions.rows[index].label.as_deref() != degenerate_label.as_deref())
            .count()
    });
    let score_nonconstant = slice_score_nonconstant + selection.score_nonconstant_taken;

    let gates = evaluate_gates(
        fixed_total > 0,
        additions.count("score"),
        &final_census,
        out_rows,
        score_nonconstant,
    );
    let manifest = serde_json::json!({
        "manifest_version": "opencodifier.corpus-merge/1",
        "temperature": {"T": TEMPERATURE_T, "family_share_cap": FAMILY_SHARE_CAP,
                        "oversample_cap": OVERSAMPLE_CAP},
        "target_total": args.target_total,
        "seed": args.seed,
        "rows_out": out_rows,
        "base": {"path": args.base.display().to_string(), "rows": base.total(),
                 "families": base_counts},
        "slices": {"count": args.slices.len(), "rows": fixed_total,
                   "families": additions_counts},
        "sample_plan": {"take": take, "shares": shares,
                        "oversample_factors": selection.oversample_factors,
                        "score_degenerate_label": degenerate_label,
                        "score_degenerate_taken": selection.score_degenerate_taken,
                        "score_nonconstant_taken": selection.score_nonconstant_taken},
        "final_families": final_census,
        "gates": gates.evaluated,
        "gates_not_applicable": gates.not_applicable,
        "mix_shares": {
            "noul": gates.noul_share,
            "score_nonconstant": gates.score_nonconstant_share
        },
        "sha256": {"rows": rows_sha256},
    });
    std::fs::write(&args.manifest, serde_json::to_string_pretty(&manifest)?)
        .map_err(|error| format!("write {}: {error}", args.manifest.display()))?;

    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "merged {out_rows} rows ({fixed_total} slice + {} sampled)",
        out_rows - fixed_total
    )?;
    writeln!(out, "final families: {final_census:?}")?;
    writeln!(
        out,
        "mix shares: noul {:.4}, score non-constant {:.4}",
        gates.noul_share, gates.score_nonconstant_share
    )?;
    writeln!(out, "gates: {:?}", gates.evaluated)?;
    for (gate, reason) in &gates.not_applicable {
        writeln!(out, "gate not applicable — {gate}: {reason}")?;
    }
    if let Some(gate) = gates.evaluated.iter().find(|(_, ok)| !*ok) {
        return Err(format!(
            "gate {} FAILED — see {} for the full census before touching the corpus",
            gate.0,
            args.manifest.display()
        )
        .into());
    }
    Ok(())
}
