# CLAUDE.md

## Project Overview

NSCB Desktop is a native Rust desktop app built with GPUI Kit (`gpui-kit` 0.9: GPUI + gpui-component) for Nintendo Switch file operations (compress, decompress, merge, convert, split, repack, rename, verify). It links nscb_rust (vendored as the workspace crate `crates/nscb`) and calls it in-process.

## Tech Stack

- **UI**: `gpui-kit` (re-exports GPUI as `gpui_kit::*`, components as `gpui_kit::component`, icons as `gpui_kit::assets`). Never depend on `gpui` directly.
- **Engine**: `crates/nscb`, a vendored copy of nscb_rust (not written by us; upstream has no license). Local changes are listed in `crates/nscb/VENDORED.md` — keep that list current
- **HTTP**: `reqwest` blocking (shared with nscb; run on `cx.background_executor()`)
- **Packaging**: `cargo-packager` (config in `[package.metadata.packager]` in `Cargo.toml`); `build.rs` embeds the Windows exe icon

## Key Architecture Decisions

- UI-independent logic lives in `src/core/` and is unit tested; UI code in `src/ui/` stays thin
- `Engine` (`src/engine.rs`) runs one invocation. `NscbEngine` parses the CLI args with `nscb::cli::Args` and calls `nscb::cli::dispatch` inside `catch_unwind`, with a `nscb::util::report::Reporter` installed to capture output lines; a poller thread reads progress bars via `report::snapshot()`
- `Runner` (`src/runner.rs`) is a GPUI entity: runs the engine on a worker thread, feeds messages through a `futures` channel, and emits `RunnerEvent`s tagged with a `Source` (the page that started the run). Pages subscribe and filter by their `Source`
- Single-file ops are batched one file per invocation; `run()` returns a `Task<RunOutcome>` (Batch Merge awaits it per game)
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
| `src/engine.rs` | `Engine` trait + `NscbEngine` (in-process nscb) |
| `src/runner.rs` | Worker-thread runner + `RunnerEvent` |
| `crates/nscb/` | Vendored nscb_rust; `src/util/report.rs` holds the embedding hooks |
| `src/core/args.rs` | `Op`, `RunOptions`, `build_args` (CLI arguments; tests ported from the old vitest suite) |
| `src/core/progress.rs` | Output-line parsing into progress/status/error |
| `src/core/batch.rs` | Batch merge scan + existing-output matching |
| `src/core/tools.rs` | Tools dir, keys import, settings, verify file list |
| `src/core/github.rs` | App update check |
| `src/ui/app.rs` | Shell: setup screen, sidebar, toasts, missing-backend banner |
| `src/ui/widgets.rs` | Shared widgets: drop zone, file list, selects, progress console (`RunView`), action bar |
| `src/ui/theme.rs` | Dark palette applied over the gpui-kit dark theme |
| `src/ui/icons.rs` | `AppIcon` SVGs (`assets/icons/`) + `AppAssets` source falling back to gpui-kit assets |
| `src/ui/pages/` | `operation.rs` (config-driven: compress/decompress/convert/split/dspl), `merge.rs`, `info.rs` (Info + Verify), `create.rs`, `rename.rs`, `batch_merge.rs`, `settings.rs` |
| `tests/ui.rs` | Headless UI integration tests (gpui-kit test harness) with a fake `Engine` |
| `scripts/portable.sh` | Portable folder assembly |

## Commands

```bash
cargo run                       # Run the app (debug)
cargo test --workspace          # Unit tests (app + nscb) + UI integration tests
cargo build --release           # Release binary
./scripts/portable.sh           # Portable folder in release/
cargo packager --release        # Bundles in target/packages/ (needs cargo-packager)
```

## nscb_rust Operations

The runner builds these CLI argument lists and hands them to `nscb::cli::dispatch` in-process:

| Operation | Args |
|---|---|
| Compress | `-z <file> [--level N] [-o dir] [--keys path]` |
| Decompress | `--decompress <file> [-o dir] [--keys path]` |
| Merge | `-d <files...> [-t xci\|nsp] [-n] [--pv] [--RSVcap N] [-k N] [-o dir] [--keys path]` |
| Convert | `-c <file> [-t xci\|nsp] [-o dir] [--keys path]` |
| Split | `--splitter <file> [-o dir] [--keys path]` |
| Split to Files | `--dspl <file> [-t xci\|nsp] [-o dir] [--keys path]` |
| Info | `--ADVcontentlist\|--ADVfilelist <file> [--keys path]` |
| Create | `--create <out.nsp> --ifolder <dir> [--keys path]` |
| Rename | `--renamef <path> [--renmode force\|skip_corr_tid\|skip_if_tid] [--addlangue true\|false] [--noversion false\|true\|xci_no_v0] [--dlcrname false\|true\|tag] [--keys path]` |
| Verify | `--verify all --text_file <list> [--vertype lv1\|lv2\|lv3] [--keys path]` |
| NUTDB Refresh | `--nutdb-refresh [--nutdb-url url]` |
| NUTDB Lookup | `--nutdb-lookup <titleID>` |

All ops except NUTDB get `--keys <tools>/prod.keys`; `--temp-dir` is added when the Temporary Directory setting is set.

## Progress

- Progress bars: nscb's `util::progress` helpers register bars with the reporter; `NscbEngine` polls `report::snapshot()` every 100 ms and forwards changes (percent capped at 99 until done)
- Output lines: percentages (`XX%`), counters, status lines (`[ACTION] detail`), `Done!` and lines containing "error" are parsed in `core::progress`

## Common Patterns

- Pages implement `FileTarget` to receive dropped/browsed files; wrap the page root with `accept_drops` and render `drop_zone`
- Selects use `widgets::Opt` items (`new_select`, `set_select`) and update page state from `SelectEvent::Confirm`
- Plain divs need `.test_support()` to be found by ID in UI tests; gpui-kit components register themselves
- In UI tests, the worker thread requires `cx.executor().allow_parking()` and real-time polling (`wait_until` in `tests/ui.rs`); `TestAppContextExt::wait_for` advances simulated time only
- In nscb code, print with `crate::outln!`/`crate::out!` (not `println!`) so embedded output reaches the app
