//! Differential conformance against ACadSharp.
//!
//! # What the oracle is, and why it is a recording
//!
//! ACadSharp is the reference implementation this crate is measured against,
//! and it is a .NET library. Running it here is not on the table, so the
//! comparison is against a RECORDING of it: `libviprs-dep` drives the real
//! ACadSharp through a flattener and commits, per fixture, the canonical
//! record dump it produced. Those dumps are the expectations. The pin in
//! `tests/acadsharp/CORPUS.pin` is what makes a run reproducible, because a
//! DWG header carries timestamps and a regenerated fixture is a different file.
//!
//! # Why this compares geometry and not the whole dump
//!
//! An expectation carries two kinds of record. Geometry records are claims
//! about the drawing: this circle, at this centre, with this radius. Warning
//! records are claims about the ACadSharp ADAPTER: which entity kinds it
//! declines to flatten, and which .NET-level reader notifications it saw. The
//! second kind cannot be true of a different implementation, and half the
//! lines in a typical expectation are one particular ACadSharp notification
//! about a table style. Diffing those would bury the signal under noise that
//! means nothing, so this reads the geometry and says so.
//!
//! # Why a baseline rather than an assertion of equality
//!
//! acadrust is not ACadSharp and this test going green was never the goal. A
//! conformance test that is red on arrival is a test nobody can use twice: the
//! second run tells you exactly what the first did. So the per-fixture verdict
//! is recorded in `tests/acadsharp/BASELINE.tsv` and this asserts against that
//! file. A fixture that gets better fails the test and asks you to re-record;
//! a fixture that gets worse fails it and names what moved. Both are the point.
//!
//! # Running it
//!
//!   scripts/fetch-acadsharp-corpus.sh
//!   ACADSHARP_CORPUS=target/acadsharp-corpus cargo test --test acadsharp_conformance
//!
//! With no corpus the test skips rather than fails, because the corpus is a
//! network fetch and a crate's test suite should not need one to be green.

mod viprs;

use std::collections::BTreeMap;
use std::path::PathBuf;

use viprs::{dump_fixture, Verdict};

fn corpus() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var("ACADSHARP_CORPUS").ok()?);
    p.join("expectations").is_dir().then_some(p)
}

const BASELINE: &str = include_str!("acadsharp/BASELINE.tsv");

fn baseline() -> BTreeMap<String, String> {
    BASELINE
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (name, verdict) = l.split_once('\t')?;
            Some((name.trim().to_string(), verdict.trim().to_string()))
        })
        .collect()
}

#[test]
fn acadrust_matches_the_acadsharp_recording() {
    let Some(corpus) = corpus() else {
        eprintln!(
            "skipping: no corpus. Run scripts/fetch-acadsharp-corpus.sh and set ACADSHARP_CORPUS."
        );
        return;
    };

    let expected = baseline();
    assert!(!expected.is_empty(), "BASELINE.tsv parsed to nothing");

    let mut actual: BTreeMap<String, String> = BTreeMap::new();
    let mut detail: BTreeMap<String, String> = BTreeMap::new();

    let mut fixtures: Vec<PathBuf> = std::fs::read_dir(corpus.join("fixtures"))
        .expect("fixtures directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "dwg"))
        .collect();
    fixtures.sort();

    for path in fixtures {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let exp = corpus.join("expectations").join(format!("{name}.txt"));
        if !exp.is_file() {
            continue; // a fixture with no recording is not a claim about anything
        }
        let v = dump_fixture(&path, &exp);
        actual.insert(name.clone(), v.tag());
        if let Some(d) = v.detail() {
            detail.insert(name, d);
        }
    }

    let mut moved = Vec::new();
    for (name, want) in &expected {
        match actual.get(name) {
            None => moved.push(format!(
                "  {name}: recorded {want}, but the run produced nothing"
            )),
            Some(got) if got != want => {
                let d = detail
                    .get(name)
                    .map(|d| format!("\n      {d}"))
                    .unwrap_or_default();
                moved.push(format!("  {name}: recorded {want}, ran {got}{d}"));
            }
            Some(_) => {}
        }
    }
    for name in actual.keys() {
        if !expected.contains_key(name) {
            moved.push(format!(
                "  {name}: ran {}, not in the baseline",
                actual[name]
            ));
        }
    }

    if !moved.is_empty() {
        panic!(
            "the ACadSharp comparison moved on {} fixture(s):\n{}\n\n\
             Re-record with: ACADSHARP_CORPUS=... cargo test --test acadsharp_conformance \
             -- --ignored record_baseline",
            moved.len(),
            moved.join("\n")
        );
    }
}

/// Rewrite `BASELINE.tsv` from a real run. Ignored by default: it is a
/// recording step, not a check, and a test that rewrites its own expectation
/// on every run asserts nothing at all.
#[test]
#[ignore]
fn record_baseline() {
    let corpus = corpus().expect("set ACADSHARP_CORPUS");
    let mut fixtures: Vec<PathBuf> = std::fs::read_dir(corpus.join("fixtures"))
        .expect("fixtures directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "dwg"))
        .collect();
    fixtures.sort();

    let mut out = String::new();
    out.push_str(
        "# Per-fixture verdict of acadrust against the recorded ACadSharp dump.\n\
         # Written by `cargo test --test acadsharp_conformance -- --ignored record_baseline`.\n\
         #\n\
         # MATCH        every geometry record agreed, to 1e-6\n\
         # DIFF:n       same record count, n records disagreed\n\
         # COUNT:a/b    acadrust produced a records, ACadSharp b\n\
         # OPEN_FAILED  acadrust refused or failed to read the file\n\
         # UNCOMPARED   the fixture needs flattening this harness does not do\n\
         #              (block expansion, hatch boundaries, dimension blocks)\n\n",
    );
    for path in &fixtures {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let exp = corpus.join("expectations").join(format!("{name}.txt"));
        if !exp.is_file() {
            continue;
        }
        out.push_str(&format!("{}\t{}\n", name, dump_fixture(path, &exp).tag()));
    }
    let dest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/acadsharp/BASELINE.tsv");
    std::fs::write(&dest, out).expect("write baseline");
    eprintln!("wrote {}", dest.display());
}

/// The report a human reads: every fixture, its verdict, and the first record
/// that disagreed. Ignored because it is output rather than a check.
#[test]
#[ignore]
fn report() {
    let corpus = corpus().expect("set ACADSHARP_CORPUS");
    let mut fixtures: Vec<PathBuf> = std::fs::read_dir(corpus.join("fixtures"))
        .expect("fixtures directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "dwg"))
        .collect();
    fixtures.sort();

    let mut tally: BTreeMap<String, usize> = BTreeMap::new();
    for path in &fixtures {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let exp = corpus.join("expectations").join(format!("{name}.txt"));
        if !exp.is_file() {
            println!("{:<26} NO_RECORDING", name);
            continue;
        }
        let v = dump_fixture(path, &exp);
        let tag = v.tag();
        *tally
            .entry(tag.split(':').next().unwrap().to_string())
            .or_default() += 1;
        println!(
            "{:<26} {:<12} {}",
            name,
            tag,
            v.detail().unwrap_or_default()
        );
    }
    println!("\n-- tally --");
    for (k, n) in tally {
        println!("{:<14} {}", k, n);
    }
}

// Verdict is re-exported for the support module's own unit tests.
#[allow(dead_code)]
fn _assert_verdict_is_used(_: Verdict) {}
