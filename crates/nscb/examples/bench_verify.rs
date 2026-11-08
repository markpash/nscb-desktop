//! Benchmark verification and its building blocks.
//!
//! ```text
//! cargo run --release -p nscb --example bench_verify -- <file.nsp|xci> --keys <prod.keys> [--level lv1|lv2|lv3]
//! ```
//!
//! Reports: raw sequential read throughput of the file, in-memory SHA-256 throughput,
//! and a full `Nscb::verify_with` run with per-stage timings and hash throughput.

use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use nscb::api::{Config, Nscb, VerifyEvent, VerifyLevel, VerifyStageKind};
use sha2::{Digest, Sha256};

fn mib_per_s(bytes: u64, elapsed: Duration) -> f64 {
    bytes as f64 / (1024.0 * 1024.0) / elapsed.as_secs_f64()
}

fn gib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

fn read_throughput(path: &PathBuf) -> (u64, Duration) {
    let mut file = std::fs::File::open(path).expect("open input");
    let mut buf = vec![0u8; 8 * 1024 * 1024];
    let mut total = 0u64;
    let start = Instant::now();
    loop {
        let n = file.read(&mut buf).expect("read input");
        if n == 0 {
            break;
        }
        total += n as u64;
    }
    (total, start.elapsed())
}

fn sha_throughput() -> (u64, Duration) {
    let buf = vec![0x5Au8; 64 * 1024 * 1024];
    let rounds = 16u64; // 1 GiB
    let start = Instant::now();
    let mut hasher = Sha256::new();
    for _ in 0..rounds {
        hasher.update(&buf);
    }
    std::hint::black_box(hasher.finalize());
    (rounds * buf.len() as u64, start.elapsed())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut input = None;
    let mut keys = None;
    let mut level = VerifyLevel::Full;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--keys" => keys = args.next().map(PathBuf::from),
            "--level" => {
                level = match args.next().as_deref() {
                    Some("lv1") => VerifyLevel::Decryption,
                    Some("lv2") => VerifyLevel::Signature,
                    _ => VerifyLevel::Full,
                }
            }
            other => input = Some(PathBuf::from(other)),
        }
    }
    let input = input.expect("usage: bench_verify <file> --keys <prod.keys> [--level lv1|lv2|lv3]");
    let size = std::fs::metadata(&input).expect("stat input").len();
    println!("file: {} ({:.2} GiB)", input.display(), gib(size));

    let (bytes, t) = read_throughput(&input);
    println!("sequential read : {:>8.1} MiB/s  ({:.2} GiB in {:.2?})", mib_per_s(bytes, t), gib(bytes), t);

    let (bytes, t) = sha_throughput();
    println!("sha256 in memory: {:>8.1} MiB/s  ({:.2} GiB in {:.2?})", mib_per_s(bytes, t), gib(bytes), t);

    let nscb = Nscb::new(Config { keys_path: keys, ..Default::default() });
    let start = Instant::now();
    let mut stage_start = start;
    let mut stage_times: Vec<(VerifyStageKind, Duration, bool)> = Vec::new();
    let mut items = 0usize;
    let report = nscb
        .verify_with(&input, level, &mut |event| match event {
            VerifyEvent::Plan(list) => items = list.len(),
            VerifyEvent::StageDone(stage) => {
                let now = Instant::now();
                stage_times.push((stage.kind, now - stage_start, stage.passed));
                stage_start = now;
            }
            VerifyEvent::Finding { .. } => {}
        })
        .expect("verify");
    let total = start.elapsed();

    println!("verify ({level:?}): {} in {total:.2?}, {items} items", if report.passed { "PASSED" } else { "FAILED" });
    for (kind, t, passed) in &stage_times {
        let extra = if *kind == VerifyStageKind::Hash {
            format!("  ~{:.1} MiB/s over the whole file", mib_per_s(size, *t))
        } else {
            String::new()
        };
        println!("  {:<11} {:>9.2?}  {}{extra}", kind.label(), t, if *passed { "ok" } else { "FAILED" });
    }
}
