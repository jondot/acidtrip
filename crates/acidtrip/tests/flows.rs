//! User flows: long multi-step scenarios in tests/flows/<area>/*.at, each
//! walking a whole task the way a person would (see tests/flows/README.md).
//! Screenshots land in target/shots/flows/<area>/<flow>/.
//!
//! A few flows run at once (`ACIDTRIP_FLOW_JOBS`, default 3); more starves
//! the debug builds. `ACIDTRIP_FLOWS=docs/` runs only flows whose path
//! contains that text. `ACIDTRIP_FLOW_BIN` runs another acidtrip binary
//! (a copy of this build) when target/ is shared with other checkouts that
//! overwrite target/debug/acidtrip.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use acidtrip_harness::{RunOptions, run_script};

const BIN: &str = env!("CARGO_BIN_EXE_acidtrip");

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|e| e == "at") {
            out.push(p);
        }
    }
}

/// `tests/flows/documents/07_foo.at` -> `/tmp/acidtrip-flows/documents-07`.
fn scratch_dir(root: &Path, flow: &Path) -> PathBuf {
    let area = flow
        .parent()
        .and_then(|d| d.strip_prefix(root).ok())
        .map(|d| d.to_string_lossy().replace('/', "-"))
        .unwrap_or_default();
    let stem = flow.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let nn = stem.split('_').next().unwrap_or(&stem);
    PathBuf::from(format!("/tmp/acidtrip-flows/{area}-{nn}"))
}

#[test]
fn user_flows() {
    let root = workspace().join("tests/flows");
    let mut flows = vec![];
    collect(&root, &mut flows);
    let only = std::env::var("ACIDTRIP_FLOWS").unwrap_or_default();
    flows.retain(|p| p.to_string_lossy().contains(&only));
    flows.sort();
    if flows.is_empty() {
        return;
    }
    // Clear only the scratch dirs of the flows about to run
    // (/tmp/acidtrip-flows/<area>-NN), so two runs at once don't wipe
    // each other's files.
    for p in &flows {
        std::fs::remove_dir_all(scratch_dir(&root, p)).ok();
    }
    std::fs::create_dir_all("/tmp/acidtrip-flows").ok();
    let bin = std::env::var_os("ACIDTRIP_FLOW_BIN").map_or_else(|| PathBuf::from(BIN), PathBuf::from);
    let jobs: usize = std::env::var("ACIDTRIP_FLOW_JOBS").ok().and_then(|s| s.parse().ok()).unwrap_or(3);
    let queue = Mutex::new(flows);
    let failures = Mutex::new(vec![]);
    std::thread::scope(|sc| {
        for _ in 0..jobs.max(1) {
            sc.spawn(|| {
                while let Some(p) = queue.lock().unwrap().pop() {
                    let rel = p.strip_prefix(&root).unwrap().with_extension("");
                    let name = rel.to_string_lossy().into_owned();
                    let shots = workspace().join("target/shots/flows").join(&rel);
                    std::fs::create_dir_all(&shots).unwrap();
                    let text = std::fs::read_to_string(&p).unwrap();
                    let mut opts = RunOptions::new(bin.clone(), shots);
                    opts.cwd = Some(workspace());
                    if let Err(e) = run_script(&text, &opts) {
                        failures.lock().unwrap().push(format!("{name}: {e:#}"));
                    }
                }
            });
        }
    });
    let mut failures = failures.into_inner().unwrap();
    failures.sort();
    assert!(failures.is_empty(), "{} flow(s) failed:\n{}", failures.len(), failures.join("\n\n"));
}
