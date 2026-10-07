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
    run(&args)
}

/// The merge itself, split out of [`main`] so the tests can drive every
/// path — including the gate-failure and bad-input exits — without
/// owning the process.
fn run(args: &Args) -> Result<(), Box<dyn Error>> {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use sha2::Digest as _;

    use super::*;

    fn next_scratch(label: &str) -> PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "opencodifier-corpus-merge-{label}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("scratch directory");
        path
    }

    /// Writes one prepped rows file; rows are JSON objects carrying at
    /// least `qtype` (the merge's only required field) and `label`.
    fn write_prepped(dir: &Path, name: &str, rows: &[serde_json::Value]) -> PathBuf {
        let path = dir.join(name);
        let mut text = String::new();
        for row in rows {
            text.push_str(&serde_json::to_string(row).expect("row serializes"));
            text.push('\n');
        }
        fs::write(&path, text).expect("rows file writes");
        path
    }

    fn prepped(qtype: &str, label: &str, tag: &str) -> serde_json::Value {
        serde_json::json!({
            "id": format!("{qtype}-{tag}"),
            "qtype": qtype,
            "label": label,
            "segments": [{"t": "x", "y": 0}],
        })
    }

    /// Base universe used by the end-to-end runs: noul 4 / choice 2 /
    /// score 10, the score bucket modal on "0" (6×) with 4 non-constant
    /// rows, mirroring the shape the gates reason about.
    fn base_rows() -> Vec<serde_json::Value> {
        let mut rows = Vec::new();
        for i in 0..4 {
            rows.push(prepped("noul", if i % 2 == 0 { "true" } else { "false" }, &format!("n{i}")));
        }
        for i in 0..2 {
            rows.push(prepped("choice", &format!("c{i}"), &format!("c{i}")));
        }
        for i in 0..6 {
            rows.push(prepped("score", "0", &format!("d{i}")));
        }
        for i in 0..4 {
            rows.push(prepped("score", "1", &format!("v{i}")));
        }
        rows
    }

    /// Score-heavy generated slice: 8 non-constant score rows + 1 noul,
    /// which is the shape that satisfies both §9.7 B6 gates on the toy
    /// base.
    fn slice_rows() -> Vec<serde_json::Value> {
        let mut rows = Vec::new();
        for i in 0..8 {
            rows.push(prepped("score", &format!("{}", (i % 3) + 1), &format!("s{i}")));
        }
        rows.push(prepped("noul", "true", "s-noul"));
        rows
    }

    fn args_for(dir: &Path, base: &[PathBuf], slices: &[PathBuf], target: usize) -> Args {
        Args {
            base: base[0].clone(),
            slices: slices.to_vec(),
            out: dir.join("rows.jsonl"),
            manifest: dir.join("manifest.json"),
            target_total: target,
            seed: 0x0C0D_1F0_2026,
        }
    }

    #[test]
    fn end_to_end_run_with_slices_passes_gates_and_writes_both_files() {
        let dir = next_scratch("e2e");
        let base = write_prepped(&dir, "base.jsonl", &base_rows());
        let slice = write_prepped(&dir, "slice.jsonl", &slice_rows());
        let args = args_for(&dir, &[base], &[slice], 40);

        run(&args).expect("merge succeeds");

        let out_text = fs::read_to_string(&args.out).expect("rows written");
        assert_eq!(out_text.lines().count(), 40, "rows_out == target");

        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&args.manifest).expect("manifest written"))
                .expect("manifest parses");
        assert_eq!(manifest["manifest_version"], "opencodifier.corpus-merge/1");
        assert_eq!(manifest["rows_out"], 40);
        assert_eq!(manifest["gates"]["noul_share_le_cap"], true);
        assert_eq!(manifest["gates"]["score_nonconstant_ge_min"], true);
        assert_eq!(manifest["gates_not_applicable"], serde_json::json!({}));
        assert_eq!(manifest["slices"]["rows"], 9);
        assert_eq!(manifest["sample_plan"]["score_degenerate_label"], "0");

        // The recorded digest is the digest of the file on disk.
        let digest = manifest["sha256"]["rows"].as_str().expect("digest recorded");
        let bytes = fs::read(&args.out).expect("rows readable");
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        assert_eq!(digest, format!("{:x}", hasher.finalize()));

        fs::remove_dir_all(&dir).expect("scratch cleans");
    }

    #[test]
    fn same_seed_reproduces_the_rows_file_byte_for_byte() {
        let dir = next_scratch("repro");
        let base = write_prepped(&dir, "base.jsonl", &base_rows());
        let slice = write_prepped(&dir, "slice.jsonl", &slice_rows());

        let mut first = args_for(&dir, &[base.clone()], &[slice.clone()], 40);
        first.out = dir.join("rows-a.jsonl");
        first.manifest = dir.join("manifest-a.json");
        run(&first).expect("first merge");
        let mut second = args_for(&dir, &[base], &[slice], 40);
        second.out = dir.join("rows-b.jsonl");
        second.manifest = dir.join("manifest-b.json");
        run(&second).expect("second merge");

        assert_eq!(
            fs::read_to_string(&first.out).expect("rows a"),
            fs::read_to_string(&second.out).expect("rows b"),
            "seeded merge is byte-reproducible"
        );
        fs::remove_dir_all(&dir).expect("scratch cleans");
    }

    #[test]
    fn slice_less_run_records_both_gates_not_applicable() {
        let dir = next_scratch("sliceless");
        let base = write_prepped(&dir, "base.jsonl", &base_rows());
        let args = args_for(&dir, &[base], &[], 20);

        run(&args).expect("slice-less merge succeeds (nothing to fail)");

        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&args.manifest).expect("manifest"))
                .expect("manifest parses");
        let reasons = manifest["gates_not_applicable"].as_object().expect("na map");
        assert_eq!(manifest["gates"], serde_json::json!({}));
        assert_eq!(reasons.len(), 2, "both gates skipped with reasons");
        for (gate, why) in reasons {
            assert!(why.as_str().expect("reason").contains("slice"), "{gate}: {why}");
        }
        fs::remove_dir_all(&dir).expect("scratch cleans");
    }

    #[test]
    fn failed_gate_errors_after_the_manifest_is_written() {
        let dir = next_scratch("gatefail");
        let base = write_prepped(&dir, "base.jsonl", &base_rows());
        // A noul-heavy slice pushes the dominant family past the 40%
        // final-mix cap.
        let noul_slice: Vec<serde_json::Value> =
            (0..20).map(|i| prepped("noul", "true", &format!("g{i}"))).collect();
        let slice = write_prepped(&dir, "slice.jsonl", &noul_slice);
        let args = args_for(&dir, &[base], &[slice], 60);

        let error = run(&args).expect_err("the noul-share gate fails");
        assert!(
            error.to_string().contains("gate noul_share_le_cap FAILED"),
            "{error}"
        );
        assert!(
            args.manifest.exists(),
            "the manifest survives the failure (evidence rule)"
        );
        fs::remove_dir_all(&dir).expect("scratch cleans");
    }

    #[test]
    fn bad_inputs_error_before_any_merge() {
        let dir = next_scratch("inputs");
        let empty = write_prepped(&dir, "empty.jsonl", &[]);
        let base = write_prepped(&dir, "base.jsonl", &base_rows());
        let slice = write_prepped(&dir, "slice.jsonl", &slice_rows());

        let error = run(&args_for(&dir, &[empty], &[], 40)).expect_err("empty base");
        assert!(error.to_string().contains("has no rows"), "{error}");

        // Slices alone meeting the target leave no base budget.
        let error = run(&args_for(&dir, &[base.clone()], &[slice], 9))
            .expect_err("slices >= target");
        assert!(error.to_string().contains("already meet the target"), "{error}");

        // A row without `qtype` is a loud load error, not a skip.
        let broken = dir.join("broken.jsonl");
        fs::write(&broken, "{\"label\": \"x\"}\n").expect("broken row writes");
        let error = run(&args_for(&dir, &[broken], &[], 40)).expect_err("missing qtype");
        assert!(error.to_string().contains("row without qtype"), "{error}");
        fs::remove_dir_all(&dir).expect("scratch cleans");
    }

    #[test]
    fn temperature_shares_cap_the_dominant_family_and_conserve_mass() {
        let mut counts = BTreeMap::new();
        counts.insert("noul".to_owned(), 100_000_usize);
        counts.insert("choice".to_owned(), 2_040_usize);
        counts.insert("score".to_owned(), 2_400_usize);
        let shares = temperature_shares(&counts);
        for (family, share) in &shares {
            assert!(
                *share <= FAMILY_SHARE_CAP + 1e-9,
                "{family} share {share} exceeds the cap"
            );
        }
        let total: f64 = shares.values().sum();
        assert!((total - 1.0).abs() < 1e-9, "shares conserve mass: {total}");
        assert!(shares["noul"] > shares["choice"], "bigger family still bigger");
    }

    #[test]
    fn take_plan_respects_the_oversample_and_never_oversample_caps() {
        let mut counts = BTreeMap::new();
        counts.insert("noul".to_owned(), 10_usize);
        counts.insert("choice".to_owned(), 2_usize);
        counts.insert("score".to_owned(), 6_usize);
        let mut shares = BTreeMap::new();
        shares.insert("noul".to_owned(), 0.6);
        shares.insert("choice".to_owned(), 0.2);
        shares.insert("score".to_owned(), 0.2);
        let take = take_plan(&counts, &shares, 100);

        assert_eq!(take["choice"], 8, "choice caps at OVERSAMPLE_CAP × count");
        assert_eq!(take["score"], 6, "score never oversamples");
        assert!(take["noul"] <= 40, "noul caps at 4× its count");
        let total: usize = take.values().sum();
        assert!(total <= 100, "the plan never exceeds the budget: {total}");
    }

    #[test]
    fn select_base_draws_score_nonconstant_first_and_records_oversampling() {
        let dir = next_scratch("select");
        let path = write_prepped(&dir, "base.jsonl", &base_rows());
        let mut bucket = Bucket::new();
        load_rows(&path, &mut bucket).expect("base loads");

        let mut take = BTreeMap::new();
        take.insert("choice".to_owned(), 4_usize); // 2 rows → 2× oversample
        take.insert("noul".to_owned(), 2_usize);
        take.insert("score".to_owned(), 6_usize);

        let mut rng = Rng::new(7);
        let selection = select_base(&bucket, &take, Some("0"), &mut rng);

        assert_eq!(selection.census["score"], 6);
        // The 4 non-constant rows are drawn before the degenerate class.
        assert_eq!(selection.score_nonconstant_taken, 4);
        assert_eq!(selection.score_degenerate_taken, 2);
        assert_eq!(selection.census["choice"], 4);
        assert_eq!(selection.oversample_factors["choice"], "2×");
        assert_eq!(selection.rows.len(), 12);
        fs::remove_dir_all(&dir).expect("scratch cleans");
    }

    #[test]
    fn evaluate_gates_applies_only_where_preconditions_hold() {
        let mut census = BTreeMap::new();
        census.insert("noul".to_owned(), 30_usize);
        census.insert("score".to_owned(), 20_usize);

        let with_slices = evaluate_gates(true, 20, &census, 100, 18);
        assert_eq!(with_slices.evaluated["noul_share_le_cap"], true);
        assert_eq!(with_slices.evaluated["score_nonconstant_ge_min"], true);
        assert!(with_slices.not_applicable.is_empty());
        assert!((with_slices.noul_share - 0.30).abs() < 1e-12);
        assert!((with_slices.score_nonconstant_share - 0.90).abs() < 1e-12);

        let over = evaluate_gates(true, 20, &census, 60, 5);
        assert_eq!(over.evaluated["noul_share_le_cap"], false, "30/60 busts the cap");
        assert_eq!(over.evaluated["score_nonconstant_ge_min"], false, "5/20 < 35%");

        let slice_less = evaluate_gates(false, 0, &census, 100, 2);
        assert!(slice_less.evaluated.is_empty());
        assert_eq!(slice_less.not_applicable.len(), 2);
    }

    #[test]
    fn load_rows_indexes_families_and_rejects_malformed_rows() {
        let dir = next_scratch("load");
        let path = write_prepped(&dir, "rows.jsonl", &base_rows());
        let mut bucket = Bucket::new();
        load_rows(&path, &mut bucket).expect("clean rows load");
        assert_eq!(bucket.total(), 16);
        assert_eq!(bucket.count("noul"), 4);
        assert_eq!(bucket.count("score"), 10);
        assert_eq!(bucket.count("choice"), 2);
        assert_eq!(bucket.family_counts()["score"], 10);

        let malformed = dir.join("malformed.jsonl");
        fs::write(&malformed, "not json\n").expect("malformed writes");
        let error = load_rows(&malformed, &mut Bucket::new()).expect_err("malformed row");
        assert!(error.to_string().contains("malformed.jsonl:1:"), "{error}");

        // Blank lines are skipped, not errors.
        let blanks = dir.join("blanks.jsonl");
        fs::write(&blanks, "\n{\"qtype\": \"noul\", \"label\": \"true\"}\n\n").expect("writes");
        let mut sparse = Bucket::new();
        load_rows(&blanks, &mut sparse).expect("blank lines skip");
        assert_eq!(sparse.total(), 1);
        fs::remove_dir_all(&dir).expect("scratch cleans");
    }

    #[test]
    fn modal_score_label_is_measured_from_the_bucket() {
        let dir = next_scratch("modal");
        let path = write_prepped(&dir, "base.jsonl", &base_rows());
        let mut bucket = Bucket::new();
        load_rows(&path, &mut bucket).expect("loads");
        assert_eq!(modal_score_label(&bucket).as_deref(), Some("0"));

        let no_score = write_prepped(
            &dir,
            "noscore.jsonl",
            &[prepped("noul", "true", "a"), prepped("choice", "x", "b")],
        );
        let mut empty_family = Bucket::new();
        load_rows(&no_score, &mut empty_family).expect("loads");
        assert_eq!(modal_score_label(&empty_family), None);
        fs::remove_dir_all(&dir).expect("scratch cleans");
    }

    #[test]
    fn write_rows_is_seeded_and_lossless() {
        let dir = next_scratch("write");
        let out = dir.join("rows.jsonl");
        let slices: Vec<String> = vec!["{\"id\": \"s1\"}".to_owned(), "{\"id\": \"s2\"}".to_owned()];
        let sampled: Vec<String> = (0..5).map(|i| format!("{{\"id\": \"b{i}\"}}")).collect();

        let mut rng = Rng::new(99);
        let first = write_rows(&out, &slices, &sampled, &mut rng).expect("writes");
        let lines: Vec<String> =
            fs::read_to_string(&out).expect("reads").lines().map(str::to_owned).collect();
        let mut ids: Vec<String> = lines
            .iter()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line)
                    .expect("row")["id"]
                    .as_str()
                    .expect("id")
                    .to_owned()
            })
            .collect();
        ids.sort();
        assert_eq!(ids.len(), 7, "every row lands exactly once");
        assert!(ids.contains(&"s1".to_owned()) && ids.contains(&"b4".to_owned()));

        let mut rng = Rng::new(99);
        let second = write_rows(&dir.join("again.jsonl"), &slices, &sampled, &mut rng)
            .expect("writes again");
        assert_eq!(first, second, "same seed, same bytes, same digest");
        fs::remove_dir_all(&dir).expect("scratch cleans");
    }
}
