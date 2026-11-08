//! Verification of NUP containers: structure (superblock, TOC SHA-256, record bounds,
//! package rules) at every level, and every file's SHA-256 at lv3. NUP content is not
//! encrypted or signed, so there is no decryption or signature stage.

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use indicatif::HumanBytes;

use crate::error::{NscbError, Result};
use crate::formats::nup::{group_digits, Codec, Nup, NupPackage, PackageKind};
use crate::ops::verify::{FindingStatus, VerifyEvent, VerifyFinding, VerifyLevel, VerifyReport, VerifyStage, VerifyStageKind};
use crate::util::{progress, report};

/// Damaged files named in a finding's detail; the stage report lists all of them.
const DETAIL_FILES: usize = 3;
/// Item name when the container itself cannot be read.
const CONTAINER_ITEM: &str = "Container";

struct StageBuilder<'a> {
    kind: VerifyStageKind,
    findings: Vec<VerifyFinding>,
    report: String,
    on_event: &'a mut dyn FnMut(VerifyEvent),
}

impl<'a> StageBuilder<'a> {
    fn new(kind: VerifyStageKind, title: &str, on_event: &'a mut dyn FnMut(VerifyEvent)) -> Self {
        Self { kind, findings: Vec::new(), report: format!("{title}\n"), on_event }
    }

    fn add(&mut self, item: String, group: Option<String>, status: FindingStatus, detail: String, extra: &[String]) {
        let tag = match status {
            FindingStatus::Ok => "OK",
            FindingStatus::Notice => "NOTICE",
            FindingStatus::Failed => "FAILED",
        };
        self.report.push_str(&format!("  [{tag}] {item}: {detail}\n"));
        for line in extra {
            self.report.push_str(&format!("      {line}\n"));
        }
        let finding = VerifyFinding { group, item, status, detail };
        (self.on_event)(VerifyEvent::Finding { stage: self.kind, index: self.findings.len(), finding: finding.clone() });
        self.findings.push(finding);
    }

    fn finish(self) -> VerifyStage {
        let passed = self.findings.iter().all(|f| f.status != FindingStatus::Failed);
        let mut report = self.report;
        report.push_str(if passed { "  Result: PASSED\n\n" } else { "  Result: FAILED\n\n" });
        let stage = VerifyStage { kind: self.kind, passed, findings: self.findings, report };
        (self.on_event)(VerifyEvent::StageDone(stage.clone()));
        stage
    }
}

/// Verify a `.nup`, reporting the packages to check, each finding and each completed
/// stage to `on_event`.
pub fn verify_nup_report_with(path: &Path, level: VerifyLevel, on_event: &mut dyn FnMut(VerifyEvent)) -> Result<VerifyReport> {
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let spinner = progress::spinner("Checking structure");
    let opened = Nup::open(path);
    spinner.finish_and_clear();

    let nup = match opened {
        Ok(nup) => nup,
        Err(NscbError::InvalidData(msg)) => {
            on_event(VerifyEvent::Plan(vec![CONTAINER_ITEM.into()]));
            let mut stage = StageBuilder::new(VerifyStageKind::Structure, &format!("NUP structure: {file_name}"), on_event);
            let detail = msg.strip_prefix(&format!("{file_name}: ")).unwrap_or(&msg).to_string();
            stage.add(CONTAINER_ITEM.into(), None, FindingStatus::Failed, detail, &[]);
            return Ok(VerifyReport { level, passed: false, stages: vec![stage.finish()] });
        }
        Err(e) => return Err(e),
    };

    on_event(VerifyEvent::Plan(nup.packages.iter().map(NupPackage::label).collect()));
    let names: Vec<Option<String>> = nup
        .packages
        .iter()
        .map(|p| nup.meta(p).ok().and_then(|m| m.title_name))
        .collect();

    // Structure: each package's tree and the container's package rules.
    let mut stage = StageBuilder::new(VerifyStageKind::Structure, &format!("NUP structure: {file_name}"), on_event);
    let has_base = nup.packages.iter().any(|p| p.kind == PackageKind::Base);
    let base_count = nup.packages.iter().filter(|p| p.kind == PackageKind::Base).count();
    let mut trees: Vec<Option<Vec<(String, u32)>>> = Vec::new();
    for (pkg, name) in nup.packages.iter().zip(&names) {
        let tree = match nup.walk(pkg.root) {
            Ok(files) => files,
            Err(e) => {
                stage.add(pkg.label(), name.clone(), FindingStatus::Failed, e.to_string(), &[]);
                trees.push(None);
                continue;
            }
        };
        let size: u64 = tree.iter().map(|(_, f)| nup.files[*f as usize].size).sum();
        let summary = format!("{} files, {}", group_digits(tree.len()), HumanBytes(size));
        let is_program = matches!(pkg.kind, PackageKind::Base | PackageKind::Update);
        let (status, detail) = if pkg.kind == PackageKind::Base && base_count > 1 {
            (FindingStatus::Failed, format!("{base_count} base packages (at most one is allowed)"))
        } else if pkg.kind == PackageKind::Update && !has_base && !pkg.standalone_update {
            (FindingStatus::Failed, "update without a base game is not marked standalone".to_string())
        } else if is_program && tree.is_empty() && pkg.kind == PackageKind::Base {
            // Packers leave an empty base record when an update holds the whole game.
            (FindingStatus::Notice, "base record with no files (the game is in an update)".to_string())
        } else if is_program && pkg.exefs.and_then(|d| nup.child_file(d, "main.npdm")).is_none() {
            (FindingStatus::Notice, format!("{summary}; no exefs/main.npdm"))
        } else {
            (FindingStatus::Ok, format!("{summary}; table of contents SHA-256 matches"))
        };
        stage.add(pkg.label(), name.clone(), status, detail, &[]);
        trees.push(Some(tree));
    }
    let mut stages = vec![stage.finish()];

    if level == VerifyLevel::Full {
        stages.push(hash_stage(&nup, &file_name, &names, &trees, on_event)?);
    }
    let passed = stages.iter().all(|s| s.passed);
    Ok(VerifyReport { level, passed, stages })
}

/// Recompute every file's SHA-256, each shared extent once, on several threads.
fn hash_stage(
    nup: &Nup,
    file_name: &str,
    names: &[Option<String>],
    trees: &[Option<Vec<(String, u32)>>],
    on_event: &mut dyn FnMut(VerifyEvent),
) -> Result<VerifyStage> {
    // Files that share an extent (dedup) share one check.
    type ExtentKey = (u64, u64, [u8; 32], u64, bool);
    let key = |f: u32| {
        let r = &nup.files[f as usize];
        (r.extent_offset, r.extent_size, r.sha256, r.size, r.codec == Codec::Zstd)
    };
    let mut extents: Vec<u32> = Vec::new();
    let mut slot_of: HashMap<ExtentKey, usize> = HashMap::new();
    for tree in trees.iter().flatten() {
        for &(_, f) in tree {
            slot_of.entry(key(f)).or_insert_with(|| {
                extents.push(f);
                extents.len() - 1
            });
        }
    }
    // Read in file order.
    let mut order: Vec<usize> = (0..extents.len()).collect();
    order.sort_by_key(|&i| nup.files[extents[i] as usize].extent_offset);

    let total: u64 = extents.iter().map(|&f| nup.files[f as usize].extent_size).sum();
    let bar = progress::file_progress(total, "Hashing files");
    let results: Mutex<Vec<Option<String>>> = Mutex::new(vec![None; extents.len()]);
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
    let outcome: std::io::Result<()> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| -> std::io::Result<()> {
                    let mut file = File::open(&nup.path)?;
                    let mut on_read = |n: u64| report::advance(n);
                    while !stop.load(Ordering::Relaxed) {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&slot) = order.get(i) else { break };
                        match nup.hash_file(&mut file, extents[slot], &mut on_read) {
                            Ok(problem) => results.lock().unwrap()[slot] = problem,
                            Err(e) => {
                                stop.store(true, Ordering::Relaxed);
                                return Err(e);
                            }
                        }
                    }
                    Ok(())
                })
            })
            .collect();
        // The first error wins; a cancellation shows up as one.
        handles.into_iter().map(|h| h.join().expect("hash worker panicked")).collect()
    });
    bar.finish_and_clear();
    outcome?;
    let results = results.into_inner().unwrap();

    let mut stage = StageBuilder::new(VerifyStageKind::Hash, &format!("NUP file hashes: {file_name}"), on_event);
    for ((pkg, name), tree) in nup.packages.iter().zip(names).zip(trees) {
        let Some(tree) = tree else {
            stage.add(pkg.label(), name.clone(), FindingStatus::Failed, "not checked: unreadable folder tree".into(), &[]);
            continue;
        };
        let damaged: Vec<String> = tree
            .iter()
            .filter_map(|(path, f)| results[slot_of[&key(*f)]].as_ref().map(|why| format!("{path} ({why})")))
            .collect();
        if damaged.is_empty() {
            let noun = if tree.len() == 1 { "file matches its" } else { "files match their" };
            stage.add(pkg.label(), name.clone(), FindingStatus::Ok, format!("{} {noun} SHA-256", group_digits(tree.len())), &[]);
        } else {
            let mut detail = format!("{} of {} files damaged: ", group_digits(damaged.len()), group_digits(tree.len()));
            detail.push_str(&damaged.iter().take(DETAIL_FILES).cloned().collect::<Vec<_>>().join(", "));
            if damaged.len() > DETAIL_FILES {
                detail.push_str(&format!(", and {} more", damaged.len() - DETAIL_FILES));
            }
            stage.add(pkg.label(), name.clone(), FindingStatus::Failed, detail, &damaged);
        }
    }
    Ok(stage.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::nup::tests::{sample, write_temp};

    fn run(bytes: &[u8], name: &str, level: VerifyLevel) -> (VerifyReport, Vec<VerifyEvent>) {
        let path = write_temp(name, bytes);
        let mut events = Vec::new();
        let report = verify_nup_report_with(&path, level, &mut |e| events.push(e)).unwrap();
        (report, events)
    }

    #[test]
    fn intact_container_passes_every_level() {
        let (report, events) = run(&sample(), "verify-ok", VerifyLevel::Full);
        assert!(report.passed, "{}", report.text());
        let kinds: Vec<_> = report.stages.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [VerifyStageKind::Structure, VerifyStageKind::Hash]);
        assert_eq!(report.stages[1].findings[0].detail, "4 files match their SHA-256");
        assert_eq!(report.stages[0].findings[0].group.as_deref(), Some("Sample Game"));
        assert!(matches!(&events[0], VerifyEvent::Plan(items) if items.len() == 3));

        let (report, _) = run(&sample(), "verify-lv1", VerifyLevel::Decryption);
        assert_eq!(report.stages.len(), 1);
        assert!(report.passed);
    }

    #[test]
    fn damaged_file_fails_the_hash_stage() {
        let mut bytes = sample();
        let nup = Nup::open(&write_temp("verify-probe", &bytes)).unwrap();
        let patch = (0..nup.files.len() as u32).find(|&f| nup.file_name(f) == "patch.bin").unwrap();
        bytes[nup.files[patch as usize].extent_offset as usize] ^= 0xFF;

        let (report, _) = run(&bytes, "verify-damaged", VerifyLevel::Full);
        assert!(!report.passed);
        assert!(report.stages[0].passed, "structure is intact");
        let failed: Vec<_> = report.stages[1].findings.iter().filter(|f| f.status == FindingStatus::Failed).collect();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].item, "update v65536 [0100000000010800]");
        assert!(failed[0].detail.contains("romfs/patch.bin (SHA-256 mismatch)"), "{}", failed[0].detail);
    }

    #[test]
    fn damaged_toc_is_a_failed_report() {
        let mut bytes = sample();
        bytes[0x80 + 0x40] ^= 1;
        let (report, _) = run(&bytes, "verify-toc", VerifyLevel::Full);
        assert!(!report.passed);
        assert_eq!(report.stages.len(), 1);
        assert_eq!(report.stages[0].findings[0].item, CONTAINER_ITEM);
        assert!(report.stages[0].findings[0].detail.contains("SHA-256"));
    }
}
