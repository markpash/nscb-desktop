# NSCB Desktop

Modern desktop GUI for Nintendo Switch game file operations: compress, decompress, merge, convert, split, repack, rename, and verify (NSP, XCI, NSZ, XCZ, NCZ).

Built in Rust with [GPUI Kit](https://gpui-kit.com) (GPUI + gpui-component). Powered by [nscb_rust](https://github.com/cxfcxf/nscb_rust), which is built into the app as a library (vendored in `crates/nscb`).

## Features

- Compress NSP/XCI to NSZ/XCZ (zstd level 1-22)
- Decompress NSZ/XCZ/NCZ back to NSP/XCI
- Merge, downgrade, or repack into one NSP/XCI (with firmware controls: RSV cap, key generation patch)
- Batch Merge: select a folder containing game subfolders (base + updates + DLC) and merge them all in one go, with skip/process controls per game
- Convert between NSP and XCI
- Split multi-title files into per-title folders or individual NSP/XCI files
- Create/Repack NSP from split folders
- Rename files using package metadata + NUTDB lookup, with Basic/Advanced controls for rename mode, language, version handling, and DLC naming
- NUTDB utilities: refresh cache, look up title IDs
- Verify container integrity (NSP, NSX, NSZ, XCI, XCZ) at three levels: decryption (lv1), signature (lv2), or full (lv3)
- View file info: titles, versions, firmware requirements, languages and contents
- Drag & drop files or folders (rename supports recursive folder input)
- Live progress with per-file status, output files you can reveal in Finder/Explorer, and structured results (title info cards, verification findings, rename results)
- Batch support for compress/decompress/convert/split/rename/verify
- Dark theme UI with SVG iconography and file type badges
- nscb_rust engine built in: a single executable, no separate backend download
- Proxy support for NUTDB downloads (URL prefix proxy for users behind firewalls)
- Configurable NUTDB source URL override
- Configurable temporary directory for large backend working files
- First-launch setup wizard for encryption keys
- App update notice when a newer NSCB Desktop release is published
- Cross-platform: Windows x86_64, Linux x86_64, macOS ARM64

## Supported Platforms

Windows x86_64, Linux x86_64, macOS ARM64.

## Prerequisites

- Switch keys file (`prod.keys` or `keys.txt`)
- **Windows**: Windows 10/11
- **Linux**: a Vulkan-capable GPU driver, Wayland or X11 (libxkbcommon)
- **macOS**: macOS 11.0+

## Usage

1. Run the app directly (standalone portable exe), or use the platform package.
2. On first launch, the setup wizard prompts you to import **encryption keys** (`prod.keys` or `keys.txt`).
3. Pick an operation from the sidebar and drop your files.

Keys and settings live in the app data folder: `~/Library/Application Support/NSCB Desktop` (macOS), `%APPDATA%\NSCB Desktop` (Windows) or `~/.local/share/nscb-desktop` (Linux). Releases up to 2.7.x kept them in the OS temp folder, where macOS and temp cleaners deleted them; they are moved over automatically on first start. Set `NSCB_DESKTOP_TOOLS_DIR` to use a different folder.

**macOS first launch:** the app isn't notarized by Apple, so macOS says it can't verify the developer. Open **System Settings → Privacy & Security** and click **Open Anyway** (once). If you still see "is damaged and can't be opened" (older builds), run `xattr -dr com.apple.quarantine "/Applications/NSCB Desktop.app"`.

If the system temporary drive is short on space, open **Settings > Tools > Temporary Directory** and select a directory on a larger drive. Leave it blank to keep using the operating system default.

### Rename Notes

- Basic and Advanced views share the same rename state.
- DLC Naming modes: `Off`, `Full name`, `Tag mode`.
- `Tag mode` appends `[DLC N]`; depending on metadata and rename mode, a resolved DLC name may still be used.
- The preview reflects backend-supported rename behavior; custom filename ordering is not currently supported.

## Development

Requires [Rust](https://rustup.rs) (stable). On Linux, install the GPUI build dependencies (see `.github/workflows/release.yml`).

```bash
cargo run                # Run the app (debug)
cargo test --workspace   # Unit tests (app + nscb)
```

## Build

```bash
# Release binary at target/release/nscb-desktop(.exe)
cargo build --release

# Portable folder at release/NSCB Desktop/
./scripts/portable.sh

# Installers / bundles (dmg, AppImage, deb, nsis) in target/packages/
cargo install cargo-packager --locked
cargo packager --release
```

## Project Layout

```text
nscb-desktop/
|- src/
|  |- main.rs              # Entry point
|  |- lib.rs               # App bootstrap: window, menus, theme, assets
|  |- jobs.rs              # Typed jobs, steps and outcomes
|  |- engine.rs            # Runs steps through the nscb library API
|  |- runner.rs            # Worker-thread runs, progress and result events
|  |- core/                # UI-independent logic (unit tested)
|  |  |- batch.rs          # Batch merge discovery and matching
|  |  |- tools.rs          # Tools dir, keys import, settings
|  |  `- github.rs         # App update check
|  `- ui/
|     |- app.rs            # Shell: setup screen, sidebar, toasts, banner
|     |- theme.rs          # Dark palette on top of the gpui-kit theme
|     |- icons.rs          # App SVG icons + asset source
|     |- widgets.rs        # Drop zone, file list, selects, etc.
|     |- job_view.rs       # Progress panel and structured result views
|     `- pages/            # One module per page
|- crates/nscb/           # Vendored nscb_rust (see crates/nscb/VENDORED.md)
|- assets/                 # SVG icons and app icons
|- build.rs                # Embeds the Windows exe icon
`- scripts/portable.sh     # Portable folder assembly
```

## License

ISC for the NSCB Desktop code. `crates/nscb` is a copy of nscb_rust, whose upstream repository does not include a license; see `crates/nscb/VENDORED.md`.
