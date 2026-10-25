//! `opencodifier lane`: the named stack presets (#191, `ESCALATION_LANES_PLAN`
//! §3). A lane is a contract, not a size — `list` prints the table,
//! `show` prints the exact config expansion, `use` writes it.

use std::path::PathBuf;

use opencodifier_config::{LANE_TABLE, RawConfig, lane_expand};

use crate::args::LaneSubcommand;
use crate::error::{CODE_UNWRITABLE_OUTPUT, CliError};
use crate::output;

/// Codes of the lane surface.
pub const CODE_UNKNOWN_LANE: &str = "lane.unknown";

/// Runs `lane`.
///
/// # Errors
///
/// [`CliError`] when the lane name is unknown or the config file cannot
/// be written.
pub(crate) fn run(command: &LaneSubcommand) -> Result<(), CliError> {
    match command {
        LaneSubcommand::List => {
            let mut rows: Vec<String> =
                vec![format!("{:<10} {:<28} {:>10}  artifacts", "lane", "tags", "acc*")];
            rows.push("-".repeat(78));
            for lane in LANE_TABLE {
                let tags = lane.tags.iter().map(ToString::to_string).collect::<Vec<_>>().join(",");
                let accuracy = lane
                    .accuracy_standing_in
                    .map_or("unmeasured".to_owned(), |a| format!("{a:.3}"));
                let artifacts = lane.expected_artifacts.join("; ");
                rows.push(format!("{:<10} {:<28} {:>10}  {artifacts}", lane.name, tags, accuracy));
            }
            rows.push(String::new());
            rows.push("acc* = the repo's closest *measured* row (docs/BENCHMARKS.md);".to_owned());
            rows.push("it stands in for the lane until the composition is measured.".to_owned());
            output::print_line(&rows.join("\n"))
        }
        LaneSubcommand::Show { name } => {
            let config = expand_or_err(name)?;
            let text = serde_json::to_string_pretty(&config)
                .map_err(|error| CliError::engine("config.serialize", error.to_string()))?;
            output::print_line(&text)
        }
        LaneSubcommand::Use { name, out } => {
            let config = expand_or_err(name)?;
            let path =
                out.clone().unwrap_or_else(|| PathBuf::from(opencodifier_config::PROJECT_FILE));
            let text = serde_json::to_string_pretty(&config)
                .map_err(|error| CliError::engine("config.serialize", error.to_string()))?;
            std::fs::write(&path, text).map_err(|error| {
                CliError::input(CODE_UNWRITABLE_OUTPUT, format!("{}: {error}", path.display()))
            })?;
            output::print_line(&format!("lane `{name}` written to {}", path.display()))?;
            output::print_line("verify the host with `opencodifier doctor` before deciding.")
        }
    }
}

fn expand_or_err(name: &str) -> Result<RawConfig, CliError> {
    lane_expand(name).map_err(|error| CliError::input(CODE_UNKNOWN_LANE, error.to_string()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn list_runs_without_error() {
        run(&LaneSubcommand::List).unwrap();
    }

    #[test]
    fn show_emits_valid_config_json() {
        let config = expand_or_err("zero").unwrap();
        let text = serde_json::to_string_pretty(&config).unwrap();
        assert!(text.contains("\"lane\""));
    }

    #[test]
    fn unknown_lane_maps_to_cli_error() {
        let error = expand_or_err("turbo").unwrap_err();
        assert_eq!(error.code(), CODE_UNKNOWN_LANE);
    }
}
