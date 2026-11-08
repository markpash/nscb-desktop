# CLAUDE.md

## Project Overview

NSCB Desktop is a native Rust desktop app built with GPUI Kit (`gpui-kit` 0.9: GPUI + gpui-component) for Nintendo Switch file operations (compress, decompress, merge, convert, split, repack, rename, verify). It drives the external `nscb_rust` CLI as a child process.

## Tech Stack

- **UI**: `gpui-kit` (re-exports GPUI as `gpui_kit::*`, components as `gpui_kit::component`, icons as `gpui_kit::assets`). Never depend on `gpui` directly.
- **HTTP**: `ureq` (blocking, run on `cx.background_executor()`)
- **Backend binary**: `nscb_rust` (not written by us), downloaded or imported at runtime into the tools dir
- **Packaging**: `cargo-packager` (config in `[package.metadata.packager]` in `Cargo.toml`); `build.rs` embeds the Windows exe icon

## Key Architecture Decisions

- UI-independent logic lives in `src/core/` and is unit tested; UI code in `src/ui/` stays thin
- `Runner` (`src/runner.rs`) is a GPUI entity: spawns the backend with `std::process`, reader threads feed a `futures` channel, and it emits `RunnerEvent`s tagged with a `Source` (the page that started the run). Pages subscribe and filter by their `Source`
- Single-file ops are batched one file per invocation; `run()` returns a `Task<RunOutcome>` (Batch Merge awaits it per game)
- Cancel bumps a generation counter and kills the process tree; a cancelled run emits no further events
- Tools dir: `<OS temp dir>/nscb-desktop-tools` (override with `NSCB_DESKTOP_TOOLS_DIR`) holds `prod.keys`, the backend binary, `settings.json`, and verify file lists. Same location as the old Tauri build
- All pages are created at startup and kept alive (state survives navigation); only the active one renders
- Drag-and-drop: page roots accept `ExternalPaths` drops; the drop zone highlights via `group_drag_over`
- File dialogs use GPUI's `prompt_for_paths` (no native filters), so selections are filtered by extension afterwards
- Native window decorations; window closes quit the app; `cmd-q` bound to `Quit`

## File Structure

| Path | Purpose |
|---|---|
| `src/lib.rs` | App bootstrap: assets, theme, menus/quit, window options |
| `src/runner.rs` | Backend process runner + `RunnerEvent` |
| `src/core/args.rs` | `Op`, `RunOptions`, `build_args` (CLI arguments; tests ported from the old vitest suite) |
| `src/core/progress.rs` | stdout/stderr parsing into progress/status/error |
| `src/core/batch.rs` | Batch merge scan + existing-output matching |
| `src/core/tools.rs` | Tools dir, keys/backend import, settings, verify file list |
| `src/core/github.rs` | Release lookups (backend + app update) and download |
| `src/ui/app.rs` | Shell: setup screen, sidebar, toasts, missing-backend banner |
| `src/ui/widgets.rs` | Shared widgets: drop zone, file list, selects, progress console (`RunView`), action bar |
| `src/ui/theme.rs` | Dark palette applied over the gpui-kit dark theme |
| `src/ui/icons.rs` | `AppIcon` SVGs (`assets/icons/`) + `AppAssets` source falling back to gpui-kit assets |
| `src/ui/pages/` | `operation.rs` (config-driven: compress/decompress/convert/split/dspl), `merge.rs`, `info.rs` (Info + Verify), `create.rs`, `rename.rs`, `batch_merge.rs`, `settings.rs` |
| `tests/ui.rs` | Headless UI integration tests (gpui-kit test harness) against a fake backend script |
| `scripts/portable.sh` | Portable folder assembly |

## Commands

```bash
cargo run                       # Run the app (debug)
cargo test                      # Unit tests + UI integration tests
cargo build --release           # Release binary
./scripts/portable.sh           # Portable folder in release/
cargo packager --release        # Bundles in target/packages/ (needs cargo-packager)
```

## nscb_rust Operations

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

## Progress Parsing

- **stdout**: percentage (`XX%`), byte/item counters, status lines (`[ACTION] detail`), `Done!`, lines containing "error"
- **stderr**: indicatif progress bars (`X.XX MiB/Y.YY GiB`, `5/12`, last occurrence wins) and `error:` messages, with ANSI escapes stripped

## Common Patterns

- Pages implement `FileTarget` to receive dropped/browsed files; wrap the page root with `accept_drops` and render `drop_zone`
- Selects use `widgets::Opt` items (`new_select`, `set_select`) and update page state from `SelectEvent::Confirm`
- Plain divs need `.test_support()` to be found by ID in UI tests; gpui-kit components register themselves
- In UI tests, real child processes require `cx.executor().allow_parking()` and real-time polling (`wait_until` in `tests/ui.rs`); `TestAppContextExt::wait_for` advances simulated time only
