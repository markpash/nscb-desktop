//! Benchmark the decompression path.
//!
//! ```text
//! cargo run --release -p nscb --example bench_decompress -- <file.nsz|xcz> [--out DIR] [--compare ORIGINAL] [--keep]
//! ```
//!
//! Runs `Nscb::decompress`, timing each phase (by progress bar), and optionally checks
//! the output is byte-identical to `ORIGINAL` (SHA-256 of both files).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nscb::api::{Config, Nscb};
use nscb::util::report::{self, Reporter};
use sha2::{Digest, Sha256};

struct Quiet;

impl Reporter for Quiet {
    fn line(&self, _text: &str) {}
}

fn gib(bytes: u64) -> f64 {
    bytes as f64 / (1u64 << 30) as f64
}

fn sha256_file(path: &Path) -> String {
    let mut file = std::fs::File::open(path).unwrap();
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 8 << 20];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut input, mut out, mut compare) = (None, PathBuf::from("target/bench-out"), None);
    let mut keep = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = PathBuf::from(args.next().unwrap()),
            "--compare" => compare = args.next().map(PathBuf::from),
            "--keep" => keep = true,
            other => input = Some(PathBuf::from(other)),
        }
    }
    let input = input.expect("usage: bench_decompress <file.nsz|xcz> [--out DIR] [--compare ORIGINAL]");
    std::fs::create_dir_all(&out).unwrap();
    let in_size = std::fs::metadata(&input).unwrap().len();
    println!("file: {} ({:.2} GiB)", input.display(), gib(in_size));

    let nscb = Nscb::new(Config::default());
    let _guard = report::install(Arc::new(Quiet));
    let done = AtomicBool::new(false);
    let start = Instant::now();
    let (result, phases) = std::thread::scope(|scope| {
        let poller = scope.spawn(|| {
            // Phase start times, keyed by progress bar message.
            let mut phases: Vec<(String, Instant)> = Vec::new();
            while !done.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(50));
                if let Some(snap) = report::snapshot() {
                    if phases.last().map(|p| &p.0) != Some(&snap.message) {
                        phases.push((snap.message.clone(), Instant::now()));
                    }
                }
            }
            phases
        });
        let result = nscb.decompress(&input, &out);
        done.store(true, Ordering::SeqCst);
        (result, poller.join().unwrap())
    });
    let total = start.elapsed();
    let end = start + total;

    let output = result.expect("decompress");
    let out_size = std::fs::metadata(&output).unwrap().len();
    let mut prev = start;
    for (i, (msg, t0)) in phases.iter().enumerate() {
        if *t0 > prev {
            println!("  {:<28} {:>8.2?}", "(setup)", *t0 - prev);
        }
        let t1 = phases.get(i + 1).map(|p| p.1).unwrap_or(end);
        println!("  {msg:<28} {:>8.2?}  {:>7.1} MiB/s of output", t1 - *t0, out_size as f64 / (1 << 20) as f64 / (t1 - *t0).as_secs_f64());
        prev = t1;
    }
    println!(
        "  total {total:.2?}, output {:.2} GiB, {:.1} MiB/s of output",
        gib(out_size),
        out_size as f64 / (1 << 20) as f64 / total.as_secs_f64()
    );

    if let Some(original) = compare {
        let same = sha256_file(&output) == sha256_file(&original);
        println!("  byte-identical to {}: {}", original.display(), if same { "yes" } else { "NO" });
    }
    if keep {
        println!("  kept {}", output.display());
    } else {
        let _ = std::fs::remove_file(output);
    }
}
