# Vendored nscb_rust

This crate is a copy of [cxfcxf/nscb_rust](https://github.com/cxfcxf/nscb_rust)
at commit `dccd87754042e425a005eb3a35a36a30b4899313` (2026-07-12), crate version 0.1.18.
NSCB Desktop links it as a library instead of running the `nscb_rust` executable.

The upstream repository does not include a license.

## Local changes

- `src/util/report.rs` (new): embedding hooks. A host installs a `Reporter` to receive
  output lines, polls progress bars with `report::snapshot()`, and cancels through
  `Reporter::cancelled()`. Without a reporter the crate behaves exactly like the CLI.
- `println!`/`print!` calls are routed through the `outln!`/`out!` macros.
- `src/util/progress.rs`: progress bars are hidden and registered for polling when embedded.
- Cancellation checkpoints in the copy, zstd and hashing loops (`util/io.rs`,
  `formats/ncz.rs`, `crypto/hash.rs`, `ops/convert.rs`, `ops/verify.rs`).
- `src/main.rs` uses the library crate instead of re-declaring its modules.
- `nscb::VERSION` constant in `src/lib.rs`.

## Updating

Copy the new upstream `src/` over this one, re-apply the changes above, and run
`cargo test --workspace`.
