# CLAUDE.md

## Project Overview

NSCB Desktop is a native Rust desktop app built with GPUI Kit (`gpui-kit` 0.9: GPUI + gpui-component) for Nintendo Switch file operations (compress, decompress, merge, convert, split, repack, rename, verify). It links nscb_rust (vendored as the workspace crate `crates/nscb`) and calls it in-process.

## Tech Stack

- **UI**: `gpui-kit` (re-exports GPUI as `gpui_kit::*`, components as `gpui_kit::component`, icons as `gpui_kit::assets`). Never depend on `gpui` directly.
- **Engine**: `crates/nscb`, a vendored copy of nscb_rust (not written by us; upstream has no license). Local changes are listed in `crates/nscb/VENDORED.md` — keep that list current
- **HTTP**: `reqwest` blocking (shared with nscb; run on `cx.background_executor()`)
- **Build config**: `.cargo/config.toml` sets `--cfg aes_armv8` on ARM64 so `aes` 0.8 uses hardware AES (keep it; Apple Silicon decryption is ~20x slower without it). Setting `RUSTFLAGS` in the environment overrides it
- **Packaging**: `cargo-packager` (config in `[package.metadata.packager]` in `Cargo.toml`); `build.rs` embeds the Windows exe icon

## Key Architecture Decisions

- UI-independent logic lives in `src/core/` and is unit tested; UI code in `src/ui/` stays thin
- Pages submit typed `Job`s (`src/jobs.rs`: `JobKind` + inputs + output dir). A job is split into `Step`s (one per input for per-input kinds; merge and NUTDB are one step) and each step produces a typed `Outcome` (output files, rename outcomes, `ContainerInfo`, `VerifyReport`, NUTDB results)
- `Engine` (`src/engine.rs`) runs one step. `NscbEngine` calls the typed `nscb::api::Nscb` methods inside `catch_unwind`, with a `nscb::util::report::Reporter` installed for cancellation; a poller thread reads progress bars via `report::snapshot()`. Operation log lines are discarded — the UI shows structured progress and results, never console output
- `Runner` (`src/runner.rs`) is a GPUI entity: runs steps on a worker thread and emits `RunnerEvent`s (`StepStarted`, `Progress`, `Partial` intermediate results such as verify `VerifyEvent`s, `StepFinished` with the result, `Finished`, `Rejected`) tagged with a `Source` (the page that started the job). Pages subscribe, filter by their `Source` and feed a `JobView`. A failed step does not stop the remaining steps; `run()` returns all step results (Batch Merge awaits them per game)
- Cancel bumps a generation counter and sets the invocation's cancel flag; nscb stops at its next checkpoint (`report::check_cancelled()` in copy/zstd/hash loops). A cancelled run emits no further events; the next run waits until the worker is idle because the reporter is process-global (one operation at a time)
- Tools dir: `<OS temp dir>/nscb-desktop-tools` (override with `NSCB_DESKTOP_TOOLS_DIR`) holds `prod.keys`, `settings.json`, and verify file lists. Same location as the old Tauri build
- All pages are created at startup and kept alive (state survives navigation); only the active one renders
- Drag-and-drop: page roots accept `ExternalPaths` drops; the drop zone highlights via `group_drag_over`
- File dialogs use GPUI's `prompt_for_paths` (no native filters), so selections are filtered by extension afterwards
- Native window decorations; window closes quit the app; `cmd-q` bound to `Quit`

## File Structure

| Path | Purpose |
|---|---|
| `src/lib.rs` | App bootstrap: assets, theme, menus/quit, window options |
| `src/jobs.rs` | `Job`/`JobKind`/`Step`/`Outcome` |
| `src/engine.rs` | `Engine` trait + `NscbEngine` (calls `nscb::api`) |
| `src/runner.rs` | Worker-thread runner + `RunnerEvent` |
| `crates/nscb/` | Vendored nscb_rust; `src/api.rs` is the typed API, `src/util/report.rs` the progress/cancel hooks |
| `src/core/batch.rs` | Batch merge scan + existing-output matching |
| `src/core/tools.rs` | Tools dir, keys import, settings, verify file list |
| `src/core/github.rs` | App update check |
| `src/ui/app.rs` | Shell: setup screen, sidebar, toasts, missing-backend banner |
| `src/ui/widgets.rs` | Shared widgets: drop zone, file list (with per-file status), selects, action bar |
| `src/ui/job_view.rs` | `JobView` job state, progress panel, output files with reveal, Info/Verify/Rename/NUTDB result views |
| `src/ui/theme.rs` | Dark palette applied over the gpui-kit dark theme |
| `src/ui/icons.rs` | `AppIcon` SVGs (`assets/icons/`) + `AppAssets` source falling back to gpui-kit assets |
| `src/ui/pages/` | `operation.rs` (config-driven: compress/decompress/convert/split/dspl), `merge.rs`, `info.rs` (Info + Verify), `create.rs`, `rename.rs`, `batch_merge.rs`, `settings.rs` |
| `scripts/portable.sh` | Portable folder assembly |

## Commands

```bash
cargo run                       # Run the app (debug)
cargo test --workspace          # Unit tests (app + nscb) + UI integration tests
cargo build --release           # Release binary
./scripts/portable.sh           # Portable folder in release/
cargo packager --release        # Bundles in target/packages/ (needs cargo-packager)
```

## nscb Library API

`nscb::api::Nscb::new(Config { keys_path, temp_dir, nutdb_url, nutdb_cache_dir })`, then:

| Method | Returns |
|---|---|
| `compress(input, out_dir, level)` / `decompress(input, out_dir)` / `convert(input, out_dir, Format)` | output file |
| `merge(inputs, out_dir, &MergeOptions)` / `create(input_dir, out_dir)` | output file |
| `split(input, out_dir)` / `split_to_files(input, out_dir, Format)` | new entries in `out_dir` |
| `rename(target, RenameOptions)` | `Vec<RenameOutcome>` |
| `inspect(input)` | `ContainerInfo` (titles, versions, firmware, contents) |
| `verify(input, VerifyLevel)` / `verify_with(.., on_stage)` | `VerifyReport` (per-stage verdicts + findings, full text for saving); `verify_with` streams `VerifyEvent`s (items to check, each finding, each completed stage) |
| `nutdb_refresh()` / `nutdb_lookup(title_id)` | `NutdbRefresh` / `TitleLookup` |

The vendored CLI (`cargo run -p nscb -- --help`) still works and prints as upstream.

## Progress

- nscb's `util::progress` helpers register bars with the reporter; `NscbEngine` polls `report::snapshot()` every 100 ms and forwards changes as `Progress { percent: Option<f32>, message }` (percent capped at 99 until the step finishes; `None` for spinners)
- Pages show an overall panel (step N of M, percent, current activity) and per-file status in the file list

## Common Patterns

- Pages implement `FileTarget` to receive dropped/browsed files; wrap the page root with `accept_drops` and render `drop_zone`
- Selects use `widgets::Opt` items (`new_select`, `set_select`) and update page state from `SelectEvent::Confirm`
- In nscb code, print with `crate::outln!`/`crate::out!` (not `println!`) so embedded use stays quiet, and expose new results through `nscb::api` rather than printed text
