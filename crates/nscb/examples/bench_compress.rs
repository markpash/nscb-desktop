//! Benchmark the compression path.
//!
//! ```text
//! cargo run --release -p nscb --example bench_compress -- <file.nsp|xci> --keys <prod.keys> \
//!     [--out DIR] [--levels 1,3,9,18,22] [--seconds 20] [--full LEVEL] [--keep]
//! ```
//!
//! - AES-CTR: throughput of the per-byte CTR transform the compressor applies before zstd.
//! - Sampled levels: runs `Nscb::compress` at each level for `--seconds`, then cancels,
//!   reporting the steady-state rate of the "Compressing" progress bar.
//! - `--full LEVEL`: a complete compress with phase timings, output size and ratio.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aes::cipher::{BlockEncrypt, KeyInit};
use aes::Aes128;
use nscb::api::{Config, Nscb};
use nscb::util::report::{self, Reporter};

fn mib(bytes: f64) -> f64 {
    bytes / (1024.0 * 1024.0)
}

/// Same algorithm as the compressor's `aes_ctr_transform_in_place`.
fn ctr_transform(cipher: &Aes128, nonce8: &[u8], file_offset: u64, data: &mut [u8]) {
    let mut cached_block_index = u64::MAX;
    let mut cached_keystream = [0u8; 16];
    for (i, byte) in data.iter_mut().enumerate() {
        let abs = file_offset + i as u64;
        let block_index = abs / 16;
        let byte_in_block = (abs % 16) as usize;
        if block_index != cached_block_index {
            let mut ctr = [0u8; 16];
            ctr[..8].copy_from_slice(nonce8);
            ctr[8..].copy_from_slice(&block_index.to_be_bytes());
            let mut blk = aes::Block::from(ctr);
            cipher.encrypt_block(&mut blk);
            cached_keystream.copy_from_slice(&blk);
            cached_block_index = block_index;
        }
        *byte ^= cached_keystream[byte_in_block];
    }
}

fn aes_ctr_throughput() -> f64 {
    let cipher = Aes128::new_from_slice(&[7u8; 16]).unwrap();
    let mut buf = vec![0u8; 256 * 1024 * 1024];
    let start = Instant::now();
    for (i, chunk) in buf.chunks_mut(1024 * 1024).enumerate() {
        ctr_transform(&cipher, &[1u8; 8], (i * 1024 * 1024) as u64, chunk);
    }
    std::hint::black_box(&buf);
    mib(buf.len() as f64) / start.elapsed().as_secs_f64()
}

/// The compressor's current bulk CTR (`nscb::crypto::aes_ctr::ctr64be_xor`).
fn aes_ctr_bulk_throughput() -> f64 {
    let cipher = Aes128::new_from_slice(&[7u8; 16]).unwrap();
    let mut buf = vec![0u8; 1024 * 1024 * 1024];
    let start = Instant::now();
    for (i, chunk) in buf.chunks_mut(1024 * 1024).enumerate() {
        nscb::crypto::aes_ctr::ctr64be_xor(&cipher, &[1u8; 8], (i * 1024 * 1024) as u64, chunk);
    }
    std::hint::black_box(&buf);
    mib(buf.len() as f64) / start.elapsed().as_secs_f64()
}

struct Deadline {
    until: Option<Instant>,
    stop: AtomicBool,
}

impl Reporter for Deadline {
    fn line(&self, _text: &str) {}
    fn cancelled(&self) -> bool {
        self.stop.load(Ordering::SeqCst) || self.until.is_some_and(|t| Instant::now() >= t)
    }
}

struct Sample {
    finished: bool,
    total: Duration,
    /// (message, steady-state MiB/s) per progress bar seen.
    phases: Vec<(String, f64, Duration)>,
}

/// Run one compress, optionally cancelling after `limit`, polling progress bars.
fn run(nscb: &Nscb, input: &PathBuf, out: &PathBuf, level: i32, limit: Option<Duration>) -> (Sample, Option<PathBuf>) {
    let start = Instant::now();
    let deadline = Arc::new(Deadline { until: limit.map(|l| start + l), stop: AtomicBool::new(false) });
    let _guard = report::install(deadline.clone());
    let done = AtomicBool::new(false);

    let (result, phases) = std::thread::scope(|scope| {
        let poller = scope.spawn(|| {
            // message -> (first seen time, first position, last time, last position)
            let mut phases: Vec<(String, Instant, u64, Instant, u64)> = Vec::new();
            while !done.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(200));
                let Some(snap) = report::snapshot() else { continue };
                let now = Instant::now();
                match phases.iter_mut().find(|p| p.0 == snap.message) {
                    Some(p) => {
                        p.3 = now;
                        p.4 = snap.position;
                    }
                    None => phases.push((snap.message.clone(), now, snap.position, now, snap.position)),
                }
            }
            phases
        });
        let result = nscb.compress(input, out, level);
        done.store(true, Ordering::SeqCst);
        (result, poller.join().unwrap())
    });

    let phases = phases
        .into_iter()
        .map(|(msg, t0, p0, t1, p1)| {
            let secs = (t1 - t0).as_secs_f64();
            let rate = if secs > 0.0 { mib((p1 - p0) as f64) / secs } else { 0.0 };
            (msg, rate, t1 - t0)
        })
        .collect();
    let finished = result.is_ok();
    let output = result.ok();
    (Sample { finished, total: start.elapsed(), phases }, output)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut input, mut keys, mut out) = (None, None, PathBuf::from("target/bench-out"));
    let mut levels = vec![1, 3, 9, 18, 22];
    let mut seconds = 20u64;
    let mut full = None;
    let mut keep = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--keys" => keys = args.next().map(PathBuf::from),
            "--out" => out = PathBuf::from(args.next().unwrap()),
            "--levels" => levels = args.next().unwrap().split(',').map(|l| l.parse().unwrap()).collect(),
            "--seconds" => seconds = args.next().unwrap().parse().unwrap(),
            "--full" => full = args.next().map(|l| l.parse::<i32>().unwrap()),
            "--keep" => keep = true,
            other => input = Some(PathBuf::from(other)),
        }
    }
    let input = input.expect("usage: bench_compress <file> --keys <prod.keys> [--out DIR] [--levels ..] [--seconds N] [--full LEVEL]");
    std::fs::create_dir_all(&out).unwrap();
    let size = std::fs::metadata(&input).unwrap().len();
    println!("file: {} ({:.2} GiB)", input.display(), size as f64 / (1u64 << 30) as f64);
    println!("aes-ctr per-byte loop (old): {:.1} MiB/s", aes_ctr_throughput());
    println!("aes-ctr bulk (current):      {:.1} MiB/s", aes_ctr_bulk_throughput());

    let nscb = Nscb::new(Config { keys_path: keys, ..Default::default() });

    if !levels.is_empty() && seconds > 0 {
        println!("\nsampled compression rate ({seconds}s per level, cancelled):");
        for level in &levels {
            let (sample, output) = run(&nscb, &input, &out, *level, Some(Duration::from_secs(seconds)));
            if let Some(o) = output {
                let _ = std::fs::remove_file(o);
            }
            let compress = sample
                .phases
                .iter()
                .find(|(m, _, _)| m.starts_with("Compressing"))
                .map(|(_, r, _)| *r)
                .unwrap_or(0.0);
            let est = if compress > 0.0 { mib(size as f64) / compress } else { f64::NAN };
            println!(
                "  level {level:>2}: {compress:>7.1} MiB/s  (full file ~{:.0}s){}",
                est,
                if sample.finished { "  [finished early]" } else { "" }
            );
        }
    }

    if let Some(level) = full {
        println!("\nfull compress at level {level}:");
        let (sample, output) = run(&nscb, &input, &out, level, None);
        for (msg, rate, t) in &sample.phases {
            println!("  {msg:<28} {:>8.2?}  {rate:>7.1} MiB/s", t);
        }
        match output {
            Some(path) => {
                let out_size = std::fs::metadata(&path).unwrap().len();
                println!(
                    "  total {:.2?}, output {:.2} GiB ({:.1}% of input)",
                    sample.total,
                    out_size as f64 / (1u64 << 30) as f64,
                    out_size as f64 / size as f64 * 100.0
                );
                if keep {
                    println!("  kept {}", path.display());
                } else {
                    let _ = std::fs::remove_file(path);
                }
            }
            None => println!("  failed after {:.2?}", sample.total),
        }
    }
}
