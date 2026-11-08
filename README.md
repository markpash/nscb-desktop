# NSCB Desktop

Modern desktop GUI for Nintendo Switch game file operations: compress, decompress, merge, convert, split, repack, rename, and verify (NSP, XCI, NSZ, XCZ, NCZ).

Built in Rust with [GPUI Kit](https://gpui-kit.com) (GPUI + gpui-component). Powered by [nscb_rust](https://github.com/cxfcxf/nscb_rust).

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
- View file info (content details and metadata summary)
- Drag & drop files or folders (rename supports recursive folder input)
- Live output console + progress tracking
- Batch support for compress/decompress/convert/split/rename/verify
- Dark theme UI with SVG iconography and file type badges
- Auto-download nscb_rust backend from GitHub releases, or import manually
- Proxy support for NUTDB downloads (URL prefix proxy for users behind firewalls)
- Configurable NUTDB source URL override
- Configurable temporary directory for large backend working files
- First-launch setup wizard for encryption keys
- App update notice when a newer NSCB Desktop release is published
- Cross-platform: Windows x86_64, Linux x86_64, macOS ARM64

## Supported Platforms

| Platform | Architecture | Backend Binary |
|---|---|---|
| Windows | x86_64 | `nscb_rust.exe` |
| Linux | x86_64 | `nscb_rust-linux-amd64` |
| macOS | ARM64 | `nscb_rust-macos-arm64` |

## Prerequisites

- Switch keys file (`prod.keys` or `keys.txt`)
- nscb_rust backend binary (auto-downloaded or imported via Settings)
- **Windows**: Windows 10/11
- **Linux**: a Vulkan-capable GPU driver, Wayland or X11 (libxkbcommon)
- **macOS**: macOS 11.0+

## Usage

1. Run the app directly (standalone portable exe), or use the platform package.
2. On first launch, the setup wizard prompts you to import **encryption keys** (`prod.keys` or `keys.txt`).
3. If the nscb_rust backend is missing, a banner directs you to **Settings > Tools** where you can download it from GitHub or import manually.
4. Pick an operation from the sidebar and drop your files.

Keys, the backend binary, and settings live in `<OS temp dir>/nscb-desktop-tools` (the same location earlier releases used, so existing setups carry over). Set `NSCB_DESKTOP_TOOLS_DIR` to use a different folder.

If the system temporary drive is short on space, open **Settings > Tools > Temporary Directory** and select a directory on a larger drive. Leave it blank to keep using the operating system default.

### Rename Notes

- Basic and Advanced views share the same rename state.
- DLC Naming modes: `Off`, `Full name`, `Tag mode`.
- `Tag mode` appends `[DLC N]`; depending on metadata and rename mode, a resolved DLC name may still be used.
- The preview reflects backend-supported rename behavior; custom filename ordering is not currently supported.

## Development

Requires [Rust](https://rustup.rs) (stable). On Linux, install the GPUI build dependencies (see `.github/workflows/release.yml`).

```bash
cargo run          # Run the app (debug)
cargo test         # Unit tests + headless UI integration tests
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
|  |- runner.rs            # Spawns nscb_rust, streams output, batches single-file ops
|  |- core/                # UI-independent logic (unit tested)
|  |  |- args.rs           # CLI argument building
|  |  |- progress.rs       # stdout/stderr progress parsing
|  |  |- batch.rs          # Batch merge discovery and matching
|  |  |- tools.rs          # Tools dir, keys/backend import, settings
|  |  `- github.rs         # Release lookups and backend download
|  `- ui/
|     |- app.rs            # Shell: setup screen, sidebar, toasts, banner
|     |- theme.rs          # Dark palette on top of the gpui-kit theme
|     |- icons.rs          # App SVG icons + asset source
|     |- widgets.rs        # Drop zone, file list, progress console, etc.
|     `- pages/            # One module per page
|- tests/ui.rs             # Headless UI tests against a fake backend
|- assets/                 # SVG icons and app icons
|- build.rs                # Embeds the Windows exe icon
`- scripts/portable.sh     # Portable folder assembly
```

## License

ISC
