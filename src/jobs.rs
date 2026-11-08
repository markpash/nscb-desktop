//! Typed jobs: what the user asked for, how it is split into engine steps, and what
//! each step produced.

use nscb::api::{
    ContainerInfo, Format, MergeOptions, NutdbRefresh, RenameOptions, RenameOutcome, TitleLookup,
    VerifyEvent, VerifyLevel, VerifyReport,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum JobKind {
    Compress { level: i32 },
    Decompress,
    Convert(Format),
    Split,
    SplitToFiles(Format),
    Merge(MergeOptions),
    Create,
    Rename(RenameOptions),
    Inspect,
    Verify(VerifyLevel),
    NutdbRefresh,
    NutdbLookup(String),
}

impl JobKind {
    /// Present-tense label for progress text, e.g. "Compressing".
    pub fn verb(&self) -> &'static str {
        match self {
            JobKind::Compress { .. } => "Compressing",
            JobKind::Decompress => "Decompressing",
            JobKind::Convert(_) => "Converting",
            JobKind::Split | JobKind::SplitToFiles(_) => "Splitting",
            JobKind::Merge(_) => "Merging",
            JobKind::Create => "Repacking",
            JobKind::Rename(_) => "Renaming",
            JobKind::Inspect => "Reading",
            JobKind::Verify(_) => "Verifying",
            JobKind::NutdbRefresh => "Refreshing NUTDB",
            JobKind::NutdbLookup(_) => "Looking up",
        }
    }

    /// Whether each input is processed by its own step. Merge combines all inputs;
    /// NUTDB operations take none.
    pub fn per_input(&self) -> bool {
        !matches!(self, JobKind::Merge(_) | JobKind::NutdbRefresh | JobKind::NutdbLookup(_))
    }

    /// Whether the job writes into an output directory.
    pub fn uses_out_dir(&self) -> bool {
        matches!(
            self,
            JobKind::Compress { .. }
                | JobKind::Decompress
                | JobKind::Convert(_)
                | JobKind::Split
                | JobKind::SplitToFiles(_)
                | JobKind::Merge(_)
                | JobKind::Create
        )
    }
}

/// A job as submitted by a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub kind: JobKind,
    pub inputs: Vec<PathBuf>,
    /// Output directory; defaults to the first input's directory.
    pub out_dir: Option<PathBuf>,
}

/// One engine invocation.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub kind: JobKind,
    pub inputs: Vec<PathBuf>,
    pub out_dir: PathBuf,
}

impl Job {
    pub fn new(kind: JobKind, inputs: Vec<PathBuf>) -> Self {
        Self { kind, inputs, out_dir: None }
    }

    pub fn with_out_dir(mut self, out_dir: Option<PathBuf>) -> Self {
        self.out_dir = out_dir;
        self
    }

    /// Split into steps: one per input for per-input kinds, otherwise a single step.
    pub fn steps(&self) -> Vec<Step> {
        let out_dir_for = |input: Option<&PathBuf>| {
            self.out_dir.clone().unwrap_or_else(|| {
                input
                    .and_then(|p| p.parent())
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from("."))
            })
        };
        if self.kind.per_input() {
            self.inputs
                .iter()
                .map(|input| Step {
                    kind: self.kind.clone(),
                    inputs: vec![input.clone()],
                    out_dir: out_dir_for(Some(input)),
                })
                .collect()
        } else {
            vec![Step {
                kind: self.kind.clone(),
                inputs: self.inputs.clone(),
                out_dir: out_dir_for(self.inputs.first()),
            }]
        }
    }
}

/// What a step produced.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Files or folders written.
    Files(Vec<PathBuf>),
    Renamed(Vec<RenameOutcome>),
    Info(ContainerInfo),
    Verified(VerifyReport),
    NutdbRefreshed(NutdbRefresh),
    Lookup(TitleLookup),
}

impl Outcome {
    pub fn files(&self) -> &[PathBuf] {
        match self {
            Outcome::Files(files) => files,
            _ => &[],
        }
    }
}

pub type StepResult = Result<Outcome, String>;

/// Intermediate results a step reports before it finishes.
#[derive(Clone, Debug, PartialEq)]
pub enum Partial {
    /// Verification progress: the items to check, findings and completed stages.
    Verify(VerifyEvent),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn per_input_jobs_get_one_step_per_file_in_its_own_directory() {
        let job = Job::new(JobKind::Compress { level: 18 }, vec![p("/a/x.nsp"), p("/b/y.xci")]);
        let steps = job.steps();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].inputs, vec![p("/a/x.nsp")]);
        assert_eq!(steps[0].out_dir, p("/a"));
        assert_eq!(steps[1].out_dir, p("/b"));
    }

    #[test]
    fn explicit_out_dir_applies_to_every_step() {
        let job = Job::new(JobKind::Decompress, vec![p("/a/x.nsz"), p("/b/y.xcz")]).with_out_dir(Some(p("/out")));
        assert!(job.steps().iter().all(|s| s.out_dir == p("/out")));
    }

    #[test]
    fn merge_and_nutdb_run_as_one_step() {
        let job = Job::new(JobKind::Merge(MergeOptions::default()), vec![p("/a/base.nsp"), p("/a/upd.nsp")]);
        let steps = job.steps();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].inputs.len(), 2);
        assert_eq!(steps[0].out_dir, p("/a"));
        assert_eq!(Job::new(JobKind::NutdbRefresh, vec![]).steps().len(), 1);
    }

    #[test]
    fn rename_and_verify_run_per_input() {
        assert!(JobKind::Rename(RenameOptions::default()).per_input());
        assert!(JobKind::Verify(VerifyLevel::Full).per_input());
        assert!(!JobKind::NutdbLookup("0100".into()).per_input());
    }
}
