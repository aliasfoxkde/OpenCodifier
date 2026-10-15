//! The benchmark-suite calibration artifacts of record must deserialize
//! into the engine's `CalibrationArtifact` (`deny_unknown_fields`) — the
//! schema gate for anything a deployment would load.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use opencodifier_engine::calibration::CalibrationArtifact;

const DIR: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/decision-model/results/calibration");

#[test]
fn every_artifact_of_record_deserializes_into_the_engine_schema() {
    let mut checked = 0;
    for entry in std::fs::read_dir(DIR).expect("calibration dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let text = std::fs::read_to_string(&path).expect("artifact text");
            let artifact: CalibrationArtifact =
                serde_json::from_str(&text).unwrap_or_else(|error| {
                    panic!("{} does not fit the engine schema: {error}", path.display())
                });
            assert!(artifact.default_temperature > 0.0);
            checked += 1;
        }
    }
    assert!(checked >= 5, "expected the D16 board's artifacts, found {checked}");
}
