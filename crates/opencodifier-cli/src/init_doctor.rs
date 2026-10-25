//! First-run detection (#192, `ESCALATION_LANES_PLAN` §5): `init` probes,
//! proposes, confirms, writes — never downloads anything. `doctor` runs
//! the same probes with no writes and exits 0/1.
//!
//! Every probe is local and read-only. The proposal is by constraint:
//! lanes are proposed by *what the host has*, and the operator confirms.

use std::path::{Path, PathBuf};

use opencodifier_config::LanePreset;

use crate::args::{DoctorArgs, InitArgs};
use crate::error::{CODE_UNWRITABLE_OUTPUT, CliError};
use crate::output;

/// Codes of the first-run surface.
pub const CODE_INIT_REFUSED: &str = "cli.init_refused";
pub const CODE_DOCTOR_FAILED: &str = "cli.doctor_failed";

/// One probe's finding.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeFinding {
    /// Probe name (`cpu`, `ram`, `gpu`, `local-server`, `artifacts`,
    /// `config`).
    pub probe: &'static str,
    /// What was found, one line.
    pub found: String,
    /// Whether the probe supports lane proposals (a failed probe does
    /// not fail `doctor` unless it is required by the active lane).
    pub ok: bool,
}

/// Probes the host. All reads, no writes, no network.
#[must_use]
pub fn probe_host() -> Vec<ProbeFinding> {
    vec![
        probe_cpu(),
        probe_ram(),
        probe_gpu(),
        probe_local_server(),
        probe_artifacts(),
        probe_config(),
    ]
}

fn probe_cpu() -> ProbeFinding {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let flags = std::fs::read_to_string("/proc/cpuinfo").map_or("unknown ISA", |text| {
        let has_avx2 = text.contains(" avx2");
        let has_avx512 = text.contains(" avx512");
        if has_avx512 {
            "AVX512"
        } else if has_avx2 {
            "AVX2"
        } else {
            "baseline"
        }
    });
    ProbeFinding { probe: "cpu", found: format!("{cores} cores, {flags}"), ok: true }
}

fn probe_ram() -> ProbeFinding {
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let total_kib = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|rest| rest.split_whitespace().next().and_then(|kib| kib.parse::<u64>().ok()));
    match total_kib {
        Some(kib) => {
            #[allow(clippy::cast_precision_loss)]
            let gib = kib as f64 / (1024.0 * 1024.0);
            let tier = if gib >= 8.0 { "2B/4B models fit" } else { "small models only" };
            ProbeFinding { probe: "ram", found: format!("{gib:.1} GiB total — {tier}"), ok: true }
        }
        None => ProbeFinding {
            probe: "ram",
            found: "unreadable (/proc/meminfo missing?)".to_owned(),
            ok: false,
        },
    }
}

fn probe_gpu() -> ProbeFinding {
    let vulkan_icd = Path::new("/usr/share/vulkan/icd.d");
    let has_vulkan = vulkan_icd.is_dir()
        && std::fs::read_dir(vulkan_icd).is_ok_and(|entries| {
            entries
                .filter_map(Result::ok)
                .any(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        });
    let has_nvidia = which_on_path("nvidia-smi").is_some();
    let found = if has_vulkan && has_nvidia {
        "Vulkan ICDs + NVIDIA tooling present"
    } else if has_vulkan {
        "Vulkan ICDs present"
    } else if has_nvidia {
        "NVIDIA tooling present (no Vulkan ICDs)"
    } else {
        "none detected — CPU-only"
    };
    ProbeFinding { probe: "gpu", found: found.to_owned(), ok: true }
}

fn probe_local_server() -> ProbeFinding {
    match which_on_path("llama-server") {
        Some(path) => ProbeFinding {
            probe: "local-server",
            found: format!("llama-server on PATH at {}", path.display()),
            ok: true,
        },
        None => ProbeFinding {
            probe: "local-server",
            found: "llama-server not on PATH — local-* lanes need it (or the fork build)"
                .to_owned(),
            ok: false,
        },
    }
}

fn probe_artifacts() -> ProbeFinding {
    let candidates = [PathBuf::from("models"), PathBuf::from("/nas/Temp/work/oc-model-eval")];
    let mut found = Vec::new();
    for dir in candidates {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "gguf")
                    && let Some(name) = path.file_name().and_then(|n| n.to_str())
                {
                    found.push(name.to_owned());
                }
            }
        }
    }
    if found.is_empty() {
        ProbeFinding {
            probe: "artifacts",
            found: "no GGUF artifacts in the known dirs".to_owned(),
            ok: false,
        }
    } else {
        found.truncate(4);
        ProbeFinding { probe: "artifacts", found: format!("GGUF: {}", found.join(", ")), ok: true }
    }
}

fn probe_config() -> ProbeFinding {
    let project = Path::new(opencodifier_config::PROJECT_FILE);
    let first_run = !project.exists();
    ProbeFinding {
        probe: "config",
        found: if first_run {
            format!("no {} — this is a first run", opencodifier_config::PROJECT_FILE)
        } else {
            format!("{} present", opencodifier_config::PROJECT_FILE)
        },
        ok: true,
    }
}

fn which_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var).map(|dir| dir.join(name)).find(|candidate| candidate.is_file())
}

/// Ranks lanes by what the host has. Returns at most three proposals,
/// each with a one-line rationale. Deterministic: same host, same order.
#[must_use]
pub fn propose_lanes(findings: &[ProbeFinding]) -> Vec<(&'static LanePreset, String)> {
    let find = |probe: &str| findings.iter().find(|f| f.probe == probe);
    let gpu = find("gpu").is_some_and(|f| f.found.contains("Vulkan") || f.found.contains("NVIDIA"));
    let server = find("local-server").is_some_and(|f| f.ok);
    let artifacts = find("artifacts").is_some_and(|f| f.ok);
    let ram_gib = find("ram")
        .and_then(|f| f.found.split_whitespace().next())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);

    let mut proposals: Vec<(&'static LanePreset, String)> = Vec::new();
    #[allow(clippy::expect_used)]
    let zero = opencodifier_config::lane_preset("zero").expect("zero is in the table");
    proposals.push((zero, "no artifacts, no model, deterministic — works on any host".to_owned()));
    #[allow(clippy::expect_used)]
    let delegate = opencodifier_config::lane_preset("delegate").expect("delegate is in the table");
    proposals.push((delegate, "the fast gate in front of a host LLM you already have".to_owned()));
    if server && artifacts && ram_gib >= 4.0 {
        #[allow(clippy::expect_used)]
        let lane = opencodifier_config::lane_preset("local-2b").expect("local-2b is in the table");
        let gpu_note = if gpu { " (GPU present: Vulkan may cut latency)" } else { "" };
        proposals.push((
            lane,
            format!("llama-server + GGUF artifacts found — self-contained accuracy{gpu_note}"),
        ));
    }
    proposals.truncate(3);
    proposals
}

/// Runs `init`.
///
/// # Errors
///
/// [`CliError`] when the proposal cannot be written or the host refuses.
pub(crate) fn run_init(args: &InitArgs) -> Result<(), CliError> {
    if let Some(name) = &args.lane {
        write_lane(name, args.out.as_deref())?;
        return Ok(());
    }
    let findings = probe_host();
    let proposals = propose_lanes(&findings);
    if args.json {
        let document = serde_json::json!({
            "probes": findings.iter().map(|f| serde_json::json!({
                "probe": f.probe, "found": f.found, "ok": f.ok,
            })).collect::<Vec<_>>(),
            "proposals": proposals.iter().map(|(lane, why)| serde_json::json!({
                "lane": lane.name, "why": why,
            })).collect::<Vec<_>>(),
        });
        let text = serde_json::to_string_pretty(&document)
            .map_err(|error| CliError::engine("init.serialize", error.to_string()))?;
        return output::print_line(&text);
    }
    let mut lines = Vec::new();
    for finding in &findings {
        lines.push(format!(
            "[{}] {}: {}",
            if finding.ok { "ok" } else { "--" },
            finding.probe,
            finding.found
        ));
    }
    lines.push(String::new());
    lines.push("Proposed lanes:".to_owned());
    for (index, (lane, why)) in proposals.iter().enumerate() {
        lines.push(format!("  {}. {} — {why}", index + 1, lane.name));
    }
    if args.yes {
        let (lane, _) = &proposals[0];
        write_lane(lane.name, args.out.as_deref())?;
        return Ok(());
    }
    lines.push(String::new());
    lines.push("run `opencodifier init --yes` to accept the top proposal,".to_owned());
    lines.push("or `opencodifier init --lane <name>` to choose.".to_owned());
    output::print_line(&lines.join("\n"))
}

fn write_lane(name: &str, out: Option<&std::path::Path>) -> Result<(), CliError> {
    let config = opencodifier_config::lane_expand(name)
        .map_err(|error| CliError::input(CODE_INIT_REFUSED, error.to_string()))?;
    let path =
        out.map_or_else(|| PathBuf::from(opencodifier_config::PROJECT_FILE), Path::to_path_buf);
    let text = serde_json::to_string_pretty(&config)
        .map_err(|error| CliError::engine("config.serialize", error.to_string()))?;
    std::fs::write(&path, text).map_err(|error| {
        CliError::input(CODE_UNWRITABLE_OUTPUT, format!("{}: {error}", path.display()))
    })?;
    output::print_line(&format!("lane `{name}` written to {}", path.display()))
}

/// Runs `doctor`: the same probes, no writes, exit 0 when healthy.
///
/// # Errors
///
/// [`CliError`] with [`CODE_DOCTOR_FAILED`] when a probe the active lane
/// requires fails.
pub(crate) fn run_doctor(_args: &DoctorArgs) -> Result<(), CliError> {
    let findings = probe_host();
    let mut lines = Vec::new();
    let mut failed = 0usize;
    for finding in &findings {
        lines.push(format!(
            "[{}] {}: {}",
            if finding.ok { "ok" } else { "FAIL" },
            finding.probe,
            finding.found
        ));
        if !finding.ok {
            failed += 1;
        }
    }
    output::print_line(&lines.join("\n"))?;
    if failed > 0 {
        return Err(CliError::engine(
            CODE_DOCTOR_FAILED,
            format!("{failed} probe(s) failed — see above"),
        ));
    }
    Ok(())
}
