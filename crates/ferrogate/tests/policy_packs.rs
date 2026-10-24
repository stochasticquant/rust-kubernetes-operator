// SPDX-License-Identifier: Apache-2.0

//! Golden tests for the policy packs in `policies/`.
//!
//! Each pack has `policy.yaml` and `fixtures/{pass,fail}/*.yaml`. A `pass` fixture must not violate
//! any rule. A `fail` fixture named `<rule>--<description>.yaml` must violate exactly `<rule>`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ferrogate::check;

fn packs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../policies")
}

fn failing_rules(decision: &ferrogate_core::Decision) -> BTreeSet<String> {
    decision
        .failures()
        .flat_map(|r| r.violations.iter().map(|v| v.rule.clone()))
        .collect()
}

#[test]
fn every_pack_matches_its_fixtures() {
    let mut packs: Vec<PathBuf> = std::fs::read_dir(packs_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("policy.yaml").exists())
        .collect();
    packs.sort();
    assert!(
        packs.len() >= 3,
        "expected the built-in packs, found {packs:?}"
    );

    let mut checked = 0;
    for pack in packs {
        let docs = check::load(&[pack.join("policy.yaml")]).unwrap();
        let policies = check::policies(&docs).unwrap();
        assert_ne!(policies.len(), 0, "{} has no policies", pack.display());

        for outcome in ["pass", "fail"] {
            let dir = pack.join("fixtures").join(outcome);
            let fixtures = check::load(std::slice::from_ref(&dir)).unwrap();
            assert_ne!(fixtures.len(), 0, "{} has no fixtures", dir.display());
            for fixture in fixtures {
                let failed = failing_rules(&fixture.evaluate(&policies));
                let file = fixture
                    .source
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                let expected: BTreeSet<String> = match outcome {
                    "pass" => BTreeSet::new(),
                    _ => BTreeSet::from([file
                        .split_once("--")
                        .map_or(file.as_str(), |(rule, _)| rule)
                        .to_owned()]),
                };
                assert_eq!(
                    failed,
                    expected,
                    "{}: {}",
                    fixture.source.display(),
                    fixture.display()
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 20, "only {checked} fixtures checked");
}

#[test]
fn samples_are_valid_policies() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/samples");
    let docs = check::load(&[dir]).unwrap();
    let policies = check::policies(&docs).unwrap();
    assert_ne!(policies.len(), 0, "no sample policies found");
}
