# Vendored nscb_rust

This crate is a copy of [cxfcxf/nscb_rust](https://github.com/cxfcxf/nscb_rust)
at commit `dccd87754042e425a005eb3a35a36a30b4899313` (2026-07-12), crate version 0.1.18.
NSCB Desktop links it as a library instead of running the `nscb_rust` executable.

The upstream repository does not include a license.

## Local changes

- `src/util/report.rs` (new): embedding hooks. A host installs a `Reporter` to receive
  output lines, polls progress bars with `report::snapshot()` (position, length and
  smoothed rate), and cancels through
  `Reporter::cancelled()`. Without a reporter the crate behaves exactly like the CLI.
- `println!`/`print!` calls are routed through the `outln!`/`out!` macros.
- `src/util/progress.rs`: progress bars are hidden and registered for polling when embedded.
- Cancellation checkpoints in the copy, zstd and hashing loops (`util/io.rs`,
  `formats/ncz.rs`, `crypto/hash.rs`, `ops/convert.rs`, `ops/verify.rs`).
- `src/main.rs` uses the library crate instead of re-declaring its modules.
- `nscb::VERSION` constant in `src/lib.rs`.
- `src/api.rs` (new): typed library API (`nscb::api::Nscb`) with one method per
  operation, returning output paths, rename outcomes, container info and verify reports.
- `src/cli.rs`: rename option types are public; renaming returns per-file
  `RenameOutcome`s (with a progress bar); merge output naming moved into
  `merge_output_path`; path helpers are `pub(crate)`. CLI output is unchanged.
- `src/ops/info.rs`: public `inspect()` returning `ContainerInfo`/`TitleInfo`.
- `src/ops/verify.rs`: public `verify_report()` returning per-stage verdicts and
  per-item findings parsed from each stage's report; the CLI's non-interactive path
  shares the same stage runner. `verify_report_with()` (and `Nscb::verify_with()`)
  streams `VerifyEvent`s: the items to check (`Plan`), each finding as its report
  line is written (`Finding`, via the `StageOut` writer and incremental
  `FindingParser`), and each completed stage (`StageDone`). When embedded, each stage
  has a progress bar (spinners for decryption/signature, a byte bar for hashing fed by
  `report::advance`).

- `Cargo.toml`: `sha2` upgraded to 0.11, which uses the CPU's SHA-256 instructions
  (ARMv8 SHA2, x86 SHA-NI) when available; 0.10's are only enabled with its `asm`
  feature. Verifying a 7.44 GiB XCI at lv3 on an M2 Max went from 17.4 s to 3.3 s.
  `sha2` 0.10 stays as `sha2_rsa` only for rsa 0.9's PSS verifier (`RsaSha256` in
  `ops/verify.rs`).
- AES-CTR: `crypto::aes_ctr::ctr64be_xor` (new, built on the `ctr` crate) replaces the
  per-byte CTR loops in `formats/ncz.rs`, `ops/compress.rs`, `ops/create.rs` and
  `ops/merge.rs` (`aes_ctr_transform_in_place`). Output is identical (unit-tested
  against the old loop). Variants with a little-endian counter option are unchanged.
- `formats/ncz.rs`: zstd compresses with one worker per CPU core (`zstd` `zstdmt`
  feature). Output is a standard zstd frame but not byte-identical to single-threaded
  output.
- Decompression is single-pass: `formats/ncz.rs` re-encrypts sections as decompressed
  bytes are written (`SectionCryptWriter`, replacing `reencrypt_sections_in_place`), and
  `ops/decompress.rs` writes the NSP/XCI header first (sizes and header hashes come
  from `NczReader::nca_size` / `nca_prefix_hash`) and decompresses each NCZ straight
  into the output, with no temp files. Progress advances by bytes written.
  `decompress_ncz` now only needs `W: Write`. 6.03 GiB XCZ -> XCI on an M2 Max:
  10.0-10.5 s -> 5.5-7.1 s; output byte-identical to the old path apart from the XCI
  header's random signature bytes.
- `ops/verify.rs`: NCZ entries are hashed while they are decompressed (`HashingWriter`,
  also for the RSV-restored hash) instead of via temp files. XCZ lv3 verify: ~13 s ->
  6.4 s; a single flipped byte in the compressed data still fails the hash stage.
- Hardware AES on ARM64 comes from the workspace `.cargo/config.toml`
  (`--cfg aes_armv8`; aes 0.8 only uses ARMv8 AES instructions with it).
- Compressing a 7.44 GiB XCI on an M2 Max: level 3 116 s -> 5.8 s, level 18 ~19 min
  -> 119 s; both outputs pass lv3 verification.
- `examples/bench_verify.rs` (new): read, SHA-256 and full-verify benchmark.
- `examples/bench_decompress.rs` (new): full decompress with phase timings and an
  optional byte comparison against the original.
- `examples/bench_compress.rs` (new): AES-CTR throughput, sampled compression rate per
  zstd level, and a full compress with phase timings.

## Updating

Copy the new upstream `src/` over this one, re-apply the changes above, and run
`cargo test --workspace`.
