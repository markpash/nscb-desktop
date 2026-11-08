//! NUP containers (`.nup`): unencrypted NX game dumps, one file per game.
//!
//! Format: <https://github.com/a55uka/nup> `SPEC.md` v1 (MIT). A NUP holds packages
//! (base game, updates, DLC), each a tree of ExeFS/RomFS files stored raw or as one
//! zstd frame, with a SHA-256 per file and over the table of contents. Nothing is
//! encrypted, so no keys are needed.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{NscbError, Result};

const SUPERBLOCK_SIZE: usize = 0x80;
const TOC_HEADER_SIZE: usize = 0x30;
const PACKAGE_SIZE: usize = 0x40;
const DIR_SIZE: usize = 0x18;
const FILE_SIZE: usize = 0x50;
const NO_DIR: u32 = 0xFFFF_FFFF;
/// Package flags bit 0: an update in a container without a base game.
const STANDALONE_UPDATE: u8 = 1;
/// Dump-tool files a producer may have kept (SPEC §6.3); not game files.
const ARTIFACTS: &[&[u8]] = &[b".listing"];
/// Deepest folder nesting followed when walking a tree.
const MAX_DEPTH: usize = 256;

/// `186592` → `186,592`, for file counts in reports.
pub(crate) fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Whether `path` starts with the NUP magic.
pub fn is_nup(path: &Path) -> bool {
    let mut magic = [0u8; 3];
    File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && &magic == b"NUP"
}

/// Whether `path` has the `.nup` extension.
pub fn has_nup_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("nup"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageKind {
    Base,
    Update,
    Addon,
    /// Reserved in v1.
    AddonUpdate,
    /// Reserved in v1.
    System,
}

impl PackageKind {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Base,
            1 => Self::Update,
            2 => Self::Addon,
            3 => Self::AddonUpdate,
            4 => Self::System,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Update => "update",
            Self::Addon => "addon",
            Self::AddonUpdate => "addon_update",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Raw,
    Zstd,
}

#[derive(Debug, Clone)]
pub struct NupPackage {
    pub index: usize,
    pub title_id: u64,
    pub root: u32,
    pub exefs: Option<u32>,
    pub romfs: Option<u32>,
    /// NCM content version, verbatim.
    pub version: u32,
    pub kind: PackageKind,
    pub standalone_update: bool,
    meta_offset: u64,
    meta_size: u32,
}

impl NupPackage {
    /// e.g. `update v131072 [0400486024DEC000]`.
    pub fn label(&self) -> String {
        format!("{} v{} [{:016X}]", self.kind.label(), self.version, self.title_id)
    }
}

#[derive(Debug, Clone, Copy)]
struct DirRecord {
    name_off: u32,
    name_len: u32,
    first_child: u32,
    child_count: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct NupFile {
    name_off: u32,
    name_len: u32,
    /// Uncompressed size.
    pub size: u64,
    pub extent_offset: u64,
    /// Stored (compressed) size.
    pub extent_size: u64,
    pub sha256: [u8; 32],
    pub codec: Codec,
}

/// A child of a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Node {
    Dir(u32),
    File(u32),
}

/// A package's meta block (SPEC §5).
#[derive(Debug, Clone, Default)]
pub struct NupMeta {
    pub npdm: Option<Vec<u8>>,
    pub nacp: Option<Vec<u8>>,
    pub icon: Option<Vec<u8>>,
    pub display_version: Option<String>,
    pub title_name: Option<String>,
    pub publisher: Option<String>,
    /// NSO ModuleId of `exefs/main`.
    pub build_id: Option<[u8; 32]>,
    pub source_dump: Option<String>,
    /// (lowercase BCP-47 tag, name).
    pub localized_titles: Vec<(String, String)>,
}

impl NupMeta {
    fn parse(bytes: &[u8]) -> Self {
        let mut meta = NupMeta::default();
        let text = |b: &[u8]| Some(String::from_utf8_lossy(b).trim().to_string()).filter(|s| !s.is_empty());
        let mut p = 0usize;
        while p + 8 <= bytes.len() {
            let kind = u16::from_le_bytes([bytes[p], bytes[p + 1]]);
            let size = u32::from_le_bytes(bytes[p + 4..p + 8].try_into().unwrap()) as usize;
            p += 8;
            let Some(payload) = bytes.get(p..p.saturating_add(size)) else { break };
            p += size;
            // First record of a type wins.
            match kind {
                0x0001 if meta.npdm.is_none() => meta.npdm = Some(payload.to_vec()),
                0x0002 if meta.nacp.is_none() => meta.nacp = Some(payload.to_vec()),
                0x0003 if meta.icon.is_none() => meta.icon = Some(payload.to_vec()),
                0x0004 if meta.display_version.is_none() => meta.display_version = text(payload),
                0x0005 if meta.title_name.is_none() => meta.title_name = text(payload),
                0x0006 if meta.publisher.is_none() => meta.publisher = text(payload),
                0x0007 if meta.build_id.is_none() => meta.build_id = payload.try_into().ok(),
                0x0008 if meta.source_dump.is_none() => meta.source_dump = text(payload),
                0x0009 if meta.localized_titles.is_empty() => {
                    meta.localized_titles = localized_records(payload)
                        .into_iter()
                        .filter_map(|(tag, data)| Some((tag, text(data)?)))
                        .collect()
                }
                _ => {}
            }
        }
        meta
    }

    /// The localized title for `tag` (exact, then primary subtag).
    pub fn localized_title(&self, tag: &str) -> Option<&str> {
        let tag = tag.to_ascii_lowercase();
        let primary = tag.split('-').next().unwrap_or_default();
        let find = |t: &str| self.localized_titles.iter().find(|(k, _)| k == t).map(|(_, v)| v.as_str());
        find(&tag).or_else(|| find(primary))
    }
}

fn localized_records(payload: &[u8]) -> Vec<(String, &[u8])> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p < payload.len() {
        let len = payload[p] as usize;
        let Some(tag) = payload.get(p + 1..p + 1 + len) else { break };
        p += 1 + len;
        let Some(size) = payload.get(p..p + 4) else { break };
        let size = u32::from_le_bytes(size.try_into().unwrap()) as usize;
        p += 4;
        let Some(data) = payload.get(p..p.saturating_add(size)) else { break };
        p += size;
        out.push((String::from_utf8_lossy(tag).to_ascii_lowercase(), data));
    }
    out
}

/// A validated NUP: superblock, TOC SHA-256 and every record's bounds (SPEC §6.1).
/// File contents are checked against their SHA-256 by [`Nup::hash_file`].
#[derive(Debug)]
pub struct Nup {
    pub path: PathBuf,
    pub file_size: u64,
    pub blob_offset: u64,
    pub uuid: [u8; 16],
    pub toc_size: u64,
    pub packages: Vec<NupPackage>,
    dirs: Vec<DirRecord>,
    pub files: Vec<NupFile>,
    children: Vec<u32>,
    pool: Vec<u8>,
}

fn invalid(path: &Path, msg: impl std::fmt::Display) -> NscbError {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    NscbError::InvalidData(format!("{name}: {msg}"))
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

impl Nup {
    /// Open and validate a NUP. Errors are `InvalidData` for malformed containers.
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        let actual_size = file.metadata()?.len();
        let mut sb = [0u8; SUPERBLOCK_SIZE];
        if actual_size < SUPERBLOCK_SIZE as u64 {
            return Err(invalid(path, "not a NUP (too short)"));
        }
        file.read_exact(&mut sb)?;
        if &sb[0..3] != b"NUP" {
            return Err(invalid(path, "not a NUP"));
        }
        if sb[3] != 1 {
            return Err(invalid(path, format!("NUP format version {}; only version 1 is supported", sb[3])));
        }
        let header_size = u16_at(&sb, 4) as u64;
        let toc_offset = u64_at(&sb, 0x08);
        let toc_size = u64_at(&sb, 0x10);
        let toc_sha: [u8; 32] = sb[0x18..0x38].try_into().unwrap();
        let blob_offset = u64_at(&sb, 0x38);
        let file_size = u64_at(&sb, 0x40);
        let package_count = u32_at(&sb, 0x48);
        let flags = u32_at(&sb, 0x4C);
        let uuid: [u8; 16] = sb[0x50..0x60].try_into().unwrap();

        if header_size < SUPERBLOCK_SIZE as u64 || toc_offset != header_size || flags != 0 {
            return Err(invalid(path, "unsupported NUP header"));
        }
        if file_size != actual_size {
            return Err(invalid(
                path,
                format!("{actual_size} bytes, but its header says {file_size} (incomplete copy or download?)"),
            ));
        }
        let toc_end = toc_offset.checked_add(toc_size).filter(|e| *e <= blob_offset);
        if toc_end.is_none() || blob_offset % 16 != 0 || blob_offset > file_size {
            return Err(invalid(path, "table of contents and data regions overlap"));
        }
        if package_count == 0 {
            return Err(invalid(path, "no packages"));
        }

        let toc_len = usize::try_from(toc_size).map_err(|_| invalid(path, "table of contents too large"))?;
        let mut toc = vec![0u8; toc_len];
        file.seek(SeekFrom::Start(toc_offset))?;
        file.read_exact(&mut toc)?;
        if Sha256::digest(&toc).as_slice() != toc_sha {
            return Err(invalid(path, "table of contents does not match its SHA-256 (damaged file)"));
        }

        let malformed = || invalid(path, "malformed table of contents");
        if toc.len() < TOC_HEADER_SIZE || &toc[0..4] != b"TOC\x01" || u32_at(&toc, 4) != package_count {
            return Err(malformed());
        }
        let dir_count = usize::try_from(u64_at(&toc, 0x08)).map_err(|_| malformed())?;
        let file_count = usize::try_from(u64_at(&toc, 0x10)).map_err(|_| malformed())?;
        let pool_off = usize::try_from(u64_at(&toc, 0x18)).map_err(|_| malformed())?;
        let pool_size = usize::try_from(u64_at(&toc, 0x20)).map_err(|_| malformed())?;
        let records_end = dir_count
            .checked_mul(DIR_SIZE)
            .zip(file_count.checked_mul(FILE_SIZE))
            .and_then(|(d, f)| d.checked_add(f))
            .and_then(|n| n.checked_add(TOC_HEADER_SIZE + package_count as usize * PACKAGE_SIZE))
            .ok_or_else(malformed)?;
        if dir_count > u32::MAX as usize
            || dir_count + file_count > u32::MAX as usize
            || records_end > toc.len()
            || pool_off < records_end
            || pool_off.checked_add(pool_size).is_none_or(|e| e > toc.len())
            || (pool_off - records_end) % 4 != 0
        {
            return Err(malformed());
        }
        let pool = toc[pool_off..pool_off + pool_size].to_vec();
        let name_ok = |off: u32, len: u32| (off as usize).checked_add(len as usize).is_some_and(|e| e <= pool.len());

        let mut p = TOC_HEADER_SIZE;
        let mut packages = Vec::with_capacity(package_count as usize);
        for index in 0..package_count as usize {
            let r = &toc[p..p + PACKAGE_SIZE];
            p += PACKAGE_SIZE;
            let kind = PackageKind::from_u8(r[0x18])
                .ok_or_else(|| invalid(path, format!("package {index} has unknown kind {}", r[0x18])))?;
            let flags = r[0x19];
            if flags & !STANDALONE_UPDATE != 0 {
                return Err(invalid(path, format!("package {index} has unknown flags {flags:#x}")));
            }
            let dir = |v: u32| (v != NO_DIR).then_some(v);
            let pkg = NupPackage {
                index,
                title_id: u64_at(r, 0),
                root: u32_at(r, 0x08),
                exefs: dir(u32_at(r, 0x0C)),
                romfs: dir(u32_at(r, 0x10)),
                version: u32_at(r, 0x14),
                kind,
                standalone_update: flags & STANDALONE_UPDATE != 0,
                meta_offset: u64_at(r, 0x20),
                meta_size: u32_at(r, 0x28),
            };
            let dir_ok = |d: u32| (d as usize) < dir_count;
            if !dir_ok(pkg.root)
                || pkg.exefs.is_some_and(|d| !dir_ok(d))
                || pkg.romfs.is_some_and(|d| !dir_ok(d))
                || (pkg.meta_size > 0 && pkg.meta_offset.checked_add(pkg.meta_size as u64).is_none_or(|e| e > file_size))
            {
                return Err(invalid(path, format!("package {index} out of bounds")));
            }
            packages.push(pkg);
        }

        let mut dirs = Vec::with_capacity(dir_count);
        for _ in 0..dir_count {
            let r = &toc[p..p + DIR_SIZE];
            p += DIR_SIZE;
            let d = DirRecord { name_off: u32_at(r, 0), name_len: u32_at(r, 4), first_child: u32_at(r, 8), child_count: u32_at(r, 12) };
            if !name_ok(d.name_off, d.name_len) {
                return Err(invalid(path, "folder name out of bounds"));
            }
            dirs.push(d);
        }

        let mut files = Vec::with_capacity(file_count);
        for _ in 0..file_count {
            let r = &toc[p..p + FILE_SIZE];
            p += FILE_SIZE;
            let codec = match (r[0x40], r[0x41]) {
                (0, 0) => Codec::Raw,
                (1, 0) => Codec::Zstd,
                (c, _) => return Err(invalid(path, format!("unsupported codec {c} (only raw and zstd are supported)"))),
            };
            let f = NupFile {
                name_off: u32_at(r, 0),
                name_len: u32_at(r, 4),
                size: u64_at(r, 0x08),
                extent_offset: u64_at(r, 0x10),
                extent_size: u64_at(r, 0x18),
                sha256: r[0x20..0x40].try_into().unwrap(),
                codec,
            };
            if !name_ok(f.name_off, f.name_len) {
                return Err(invalid(path, "file name out of bounds"));
            }
            let in_blob = f.extent_offset >= blob_offset
                && f.extent_offset % 16 == 0
                && f.extent_offset.checked_add(f.extent_size).is_some_and(|e| e <= file_size);
            let ok = if f.size == 0 {
                f.extent_offset == 0 && f.extent_size == 0 && codec == Codec::Raw
            } else {
                in_blob && f.extent_size > 0 && (codec != Codec::Raw || f.extent_size == f.size)
            };
            if !ok {
                let name = String::from_utf8_lossy(&pool[f.name_off as usize..(f.name_off + f.name_len) as usize]).into_owned();
                return Err(invalid(path, format!("file {name:?} out of bounds")));
            }
            files.push(f);
        }

        let children: Vec<u32> = toc[p..pool_off].chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        let nodes = (dir_count + file_count) as u32;
        if dirs.iter().any(|d| (d.first_child as usize).checked_add(d.child_count as usize).is_none_or(|e| e > children.len()))
            || children.iter().any(|&c| c >= nodes)
        {
            return Err(invalid(path, "folder children out of bounds"));
        }

        Ok(Nup { path: path.to_path_buf(), file_size, blob_offset, uuid, toc_size, packages, dirs, files, children, pool })
    }

    pub fn dir_count(&self) -> usize {
        self.dirs.len()
    }

    pub fn dir_name(&self, d: u32) -> String {
        let r = &self.dirs[d as usize];
        String::from_utf8_lossy(&self.pool[r.name_off as usize..(r.name_off + r.name_len) as usize]).into_owned()
    }

    fn file_name_bytes(&self, f: u32) -> &[u8] {
        let r = &self.files[f as usize];
        &self.pool[r.name_off as usize..(r.name_off + r.name_len) as usize]
    }

    pub fn file_name(&self, f: u32) -> String {
        String::from_utf8_lossy(self.file_name_bytes(f)).into_owned()
    }

    /// A folder's children, artifacts (`.listing`) left out.
    pub fn children(&self, d: u32) -> impl Iterator<Item = Node> + '_ {
        let r = self.dirs[d as usize];
        let ndirs = self.dirs.len() as u32;
        self.children[r.first_child as usize..(r.first_child + r.child_count) as usize]
            .iter()
            .map(move |&c| if c < ndirs { Node::Dir(c) } else { Node::File(c - ndirs) })
            .filter(|n| !matches!(n, Node::File(f) if ARTIFACTS.contains(&self.file_name_bytes(*f))))
    }

    /// The file named `name` directly in folder `d`.
    pub fn child_file(&self, d: u32, name: &str) -> Option<u32> {
        self.children(d).find_map(|n| match n {
            Node::File(f) if self.file_name_bytes(f) == name.as_bytes() => Some(f),
            _ => None,
        })
    }

    /// Every file under folder `d` with its path relative to `d`. Errors on a folder
    /// that contains itself or nests deeper than any real dump.
    pub fn walk(&self, d: u32) -> Result<Vec<(String, u32)>> {
        let mut out = Vec::new();
        let mut on_path = HashSet::new();
        self.walk_into(d, "", 0, &mut on_path, &mut out)?;
        Ok(out)
    }

    fn walk_into(&self, d: u32, prefix: &str, depth: usize, on_path: &mut HashSet<u32>, out: &mut Vec<(String, u32)>) -> Result<()> {
        if depth > MAX_DEPTH || !on_path.insert(d) {
            return Err(invalid(&self.path, format!("folder {:?} contains itself", self.dir_name(d))));
        }
        for node in self.children(d) {
            match node {
                Node::File(f) => out.push((format!("{prefix}{}", self.file_name(f)), f)),
                Node::Dir(c) => self.walk_into(c, &format!("{prefix}{}/", self.dir_name(c)), depth + 1, on_path, out)?,
            }
        }
        on_path.remove(&d);
        Ok(())
    }

    /// A package's meta block.
    pub fn meta(&self, pkg: &NupPackage) -> Result<NupMeta> {
        if pkg.meta_size == 0 {
            return Ok(NupMeta::default());
        }
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(pkg.meta_offset))?;
        let mut bytes = vec![0u8; pkg.meta_size as usize];
        file.read_exact(&mut bytes)?;
        Ok(NupMeta::parse(&bytes))
    }

    /// The package that holds application metadata: the base record, else the first
    /// package (SPEC §5.1).
    pub fn app_package(&self) -> &NupPackage {
        self.packages.iter().find(|p| p.kind == PackageKind::Base).unwrap_or(&self.packages[0])
    }

    /// The first `n` bytes of a file (fewer for a shorter file), not hash-checked.
    pub fn read_head(&self, f: u32, n: usize) -> io::Result<Vec<u8>> {
        let rec = self.files[f as usize];
        let want = n.min(rec.size as usize);
        let mut out = vec![0u8; want];
        if want == 0 {
            return Ok(out);
        }
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(rec.extent_offset))?;
        let extent = file.take(rec.extent_size);
        match rec.codec {
            Codec::Raw => BufReader::new(extent).read_exact(&mut out)?,
            Codec::Zstd => zstd::stream::read::Decoder::new(extent)?.read_exact(&mut out)?,
        }
        Ok(out)
    }

    /// The build ID (NSO ModuleId) of a package's `exefs/main`: its meta record, else
    /// bytes 0x40..0x60 of the file.
    pub fn build_id(&self, pkg: &NupPackage, meta: &NupMeta) -> Option<[u8; 32]> {
        meta.build_id.or_else(|| {
            let main = self.child_file(pkg.exefs?, "main")?;
            let head = self.read_head(main, 0x60).ok()?;
            let magic = head.get(0..4)?;
            (magic == b"NSO0" || magic == b"NRO0").then(|| head[0x40..0x60].try_into().ok()).flatten()
        })
    }

    /// Recompute a file's SHA-256 from `file` (an open handle on this container).
    /// `on_read` receives the stored bytes read as they are read. Returns why the
    /// content does not match, or `None` when it does. I/O errors (including
    /// cancellation) are returned as errors; damaged zstd data is a mismatch.
    pub fn hash_file(&self, file: &mut File, f: u32, on_read: &mut dyn FnMut(u64)) -> io::Result<Option<String>> {
        use crate::util::report::check_cancelled;
        let rec = self.files[f as usize];
        if rec.size == 0 {
            let empty = Sha256::digest([]);
            return Ok((empty.as_slice() != rec.sha256).then(|| "SHA-256 mismatch".to_string()));
        }
        file.seek(SeekFrom::Start(rec.extent_offset))?;
        let mut source = CountingReader { inner: file.take(rec.extent_size), on_read, error: None };
        let mut hasher = Sha256::new();
        let mut total = 0u64;
        let mut buf = vec![0u8; 1 << 20];
        let mut fill = |reader: &mut dyn Read| -> io::Result<()> {
            loop {
                check_cancelled()?;
                let n = reader.read(&mut buf)?;
                if n == 0 {
                    return Ok(());
                }
                hasher.update(&buf[..n]);
                total += n as u64;
                if total > rec.size {
                    return Ok(());
                }
            }
        };
        let decoded = match rec.codec {
            Codec::Raw => fill(&mut source),
            Codec::Zstd => zstd::stream::read::Decoder::with_buffer(BufReader::with_capacity(1 << 20, &mut source))
                .and_then(|mut d| fill(&mut d)),
        };
        if let Some(e) = source.error.take() {
            return Err(e);
        }
        if let Err(e) = decoded {
            check_cancelled()?;
            return Ok(Some(format!("damaged compressed data ({e})")));
        }
        if total != rec.size {
            return Ok(Some(format!("size {total}, expected {}", rec.size)));
        }
        Ok((hasher.finalize().as_slice() != rec.sha256).then(|| "SHA-256 mismatch".to_string()))
    }
}

/// Reports bytes read and keeps the underlying I/O error (to tell it apart from a
/// decoding error).
struct CountingReader<'a, R> {
    inner: R,
    on_read: &'a mut dyn FnMut(u64),
    error: Option<io::Error>,
}

impl<R: Read> Read for CountingReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.inner.read(buf) {
            Ok(n) => {
                (self.on_read)(n as u64);
                Ok(n)
            }
            Err(e) => {
                let copy = io::Error::new(e.kind(), e.to_string());
                self.error = Some(e);
                Err(copy)
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// One file of a test container: path under the package root, content, codec.
    pub(crate) struct TestFile<'a> {
        pub path: &'a str,
        pub data: &'a [u8],
        pub zstd: bool,
    }

    pub(crate) struct TestPackage<'a> {
        pub title_id: u64,
        pub version: u32,
        pub kind: u8,
        pub flags: u8,
        pub files: Vec<TestFile<'a>>,
        /// (TLV type, payload).
        pub meta: Vec<(u16, Vec<u8>)>,
    }

    #[derive(Default)]
    struct Tree {
        dirs: Vec<String>,
        files: Vec<(String, Vec<u8>, bool)>,
        dir_children: Vec<Vec<(String, bool, usize)>>,
    }

    /// Build a NUP v1 container in memory, following SPEC.md.
    pub(crate) fn build(packages: &[TestPackage]) -> Vec<u8> {
        let mut tree = Tree::default();
        let mut pkg_dirs = Vec::new();
        let new_dir = |tree: &mut Tree, name: &str| {
            tree.dirs.push(name.to_string());
            tree.dir_children.push(Vec::new());
            tree.dirs.len() - 1
        };
        for pkg in packages {
            let root = new_dir(&mut tree, "");
            for f in &pkg.files {
                let parts: Vec<&str> = f.path.split('/').collect();
                let mut d = root;
                for part in &parts[..parts.len() - 1] {
                    let existing = tree.dir_children[d].iter().find(|(n, is_dir, _)| *is_dir && n == part).map(|c| c.2);
                    d = match existing {
                        Some(c) => c,
                        None => {
                            let c = new_dir(&mut tree, part);
                            tree.dir_children[d].push((part.to_string(), true, c));
                            c
                        }
                    };
                }
                tree.files.push((parts[parts.len() - 1].to_string(), f.data.to_vec(), f.zstd));
                let fi = tree.files.len() - 1;
                tree.dir_children[d].push((parts[parts.len() - 1].to_string(), false, fi));
            }
            let find = |tree: &Tree, name: &str| {
                tree.dir_children[root].iter().find(|(n, is_dir, _)| *is_dir && n == name).map(|c| c.2 as u32).unwrap_or(NO_DIR)
            };
            pkg_dirs.push((root as u32, find(&tree, "exefs"), find(&tree, "romfs")));
        }
        let ndirs = tree.dirs.len();

        let mut pool = Vec::new();
        let mut intern = |s: &str| {
            let off = pool.len() as u32;
            pool.extend_from_slice(s.as_bytes());
            (off, s.len() as u32)
        };
        let mut children = Vec::new();
        let mut dir_recs = Vec::new();
        for (d, name) in tree.dirs.iter().enumerate() {
            let mut kids = tree.dir_children[d].clone();
            kids.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            let first = children.len() as u32;
            for (_, is_dir, i) in &kids {
                children.push(if *is_dir { *i as u32 } else { (ndirs + i) as u32 });
            }
            dir_recs.push((intern(name), first, kids.len() as u32));
        }
        let file_names: Vec<(u32, u32)> = tree.files.iter().map(|(n, _, _)| intern(n)).collect();

        let toc_len = TOC_HEADER_SIZE + packages.len() * PACKAGE_SIZE + ndirs * DIR_SIZE + tree.files.len() * FILE_SIZE + children.len() * 4;
        let pool_off = toc_len;
        let toc_size = toc_len + pool.len();
        let blob_offset = (SUPERBLOCK_SIZE + toc_size).div_ceil(16) * 16;

        // Blob: extents, then meta blocks.
        let mut blob = Vec::new();
        let mut file_recs = Vec::new();
        for (i, (_, data, z)) in tree.files.iter().enumerate() {
            let sha: [u8; 32] = Sha256::digest(data).into();
            if data.is_empty() {
                file_recs.push((file_names[i], 0u64, 0u64, 0u64, sha, 0u8));
                continue;
            }
            let stored = if *z { zstd::bulk::compress(data, 3).unwrap() } else { data.clone() };
            while blob.len() % 16 != 0 {
                blob.push(0);
            }
            let off = (blob_offset + blob.len()) as u64;
            blob.extend_from_slice(&stored);
            file_recs.push((file_names[i], data.len() as u64, off, stored.len() as u64, sha, *z as u8));
        }
        let mut metas = Vec::new();
        for pkg in packages {
            if pkg.meta.is_empty() {
                metas.push((0u64, 0u32));
                continue;
            }
            let mut m = Vec::new();
            for (t, payload) in &pkg.meta {
                m.extend_from_slice(&t.to_le_bytes());
                m.extend_from_slice(&0u16.to_le_bytes());
                m.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                m.extend_from_slice(payload);
            }
            while blob.len() % 16 != 0 {
                blob.push(0);
            }
            metas.push(((blob_offset + blob.len()) as u64, m.len() as u32));
            blob.extend_from_slice(&m);
        }

        let mut toc = Vec::with_capacity(toc_size);
        toc.extend_from_slice(b"TOC\x01");
        toc.extend_from_slice(&(packages.len() as u32).to_le_bytes());
        toc.extend_from_slice(&(ndirs as u64).to_le_bytes());
        toc.extend_from_slice(&(tree.files.len() as u64).to_le_bytes());
        toc.extend_from_slice(&(pool_off as u64).to_le_bytes());
        toc.extend_from_slice(&(pool.len() as u64).to_le_bytes());
        toc.extend_from_slice(&0u64.to_le_bytes());
        for (pkg, ((root, exefs, romfs), (moff, msize))) in packages.iter().zip(pkg_dirs.iter().zip(&metas)) {
            toc.extend_from_slice(&pkg.title_id.to_le_bytes());
            for v in [*root, *exefs, *romfs, pkg.version] {
                toc.extend_from_slice(&v.to_le_bytes());
            }
            toc.extend_from_slice(&[pkg.kind, pkg.flags, 0, 0, 0, 0, 0, 0]);
            toc.extend_from_slice(&moff.to_le_bytes());
            toc.extend_from_slice(&msize.to_le_bytes());
            toc.extend_from_slice(&[0u8; 20]);
        }
        for ((off, len), first, count) in &dir_recs {
            for v in [*off, *len, *first, *count] {
                toc.extend_from_slice(&v.to_le_bytes());
            }
            toc.extend_from_slice(&0u64.to_le_bytes());
        }
        for ((off, len), size, eoff, esz, sha, codec) in &file_recs {
            toc.extend_from_slice(&off.to_le_bytes());
            toc.extend_from_slice(&len.to_le_bytes());
            for v in [*size, *eoff, *esz] {
                toc.extend_from_slice(&v.to_le_bytes());
            }
            toc.extend_from_slice(sha);
            toc.extend_from_slice(&[*codec, 0, 0, 0]);
            toc.extend_from_slice(&[0u8; 12]);
        }
        for c in &children {
            toc.extend_from_slice(&c.to_le_bytes());
        }
        toc.extend_from_slice(&pool);
        assert_eq!(toc.len(), toc_size);

        let file_size = (blob_offset + blob.len()) as u64;
        let mut out = Vec::with_capacity(file_size as usize);
        out.extend_from_slice(b"NUP\x01");
        out.extend_from_slice(&0x80u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&0x80u64.to_le_bytes());
        out.extend_from_slice(&(toc_size as u64).to_le_bytes());
        out.extend_from_slice(&Sha256::digest(&toc));
        out.extend_from_slice(&(blob_offset as u64).to_le_bytes());
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&(packages.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&[0u8; 16]);
        out.extend_from_slice(&[0u8; 32]);
        out.extend_from_slice(&toc);
        out.resize(blob_offset, 0);
        out.extend_from_slice(&blob);
        out
    }

    pub(crate) fn nso_main(build_id: u8) -> Vec<u8> {
        let mut nso = vec![0u8; 0x100];
        nso[0..4].copy_from_slice(b"NSO0");
        nso[0x40..0x60].fill(build_id);
        nso
    }

    pub(crate) fn sample() -> Vec<u8> {
        let main = nso_main(0xAB);
        let big = vec![7u8; 300_000];
        build(&[
            TestPackage {
                title_id: 0x0100_0000_0001_0000,
                version: 0,
                kind: 0,
                flags: 0,
                files: vec![
                    TestFile { path: "exefs/main", data: &main, zstd: true },
                    TestFile { path: "exefs/main.npdm", data: b"META", zstd: false },
                    TestFile { path: "romfs/data/big.bin", data: &big, zstd: true },
                    TestFile { path: "romfs/empty", data: b"", zstd: false },
                    TestFile { path: "romfs/.listing", data: b"ftp", zstd: false },
                ],
                meta: vec![(5, b"Sample Game".to_vec()), (6, b"Studio".to_vec()), (4, b"1.0.0".to_vec())],
            },
            TestPackage {
                title_id: 0x0100_0000_0001_0800,
                version: 65536,
                kind: 1,
                flags: 0,
                files: vec![TestFile { path: "romfs/patch.bin", data: b"patched", zstd: false }],
                meta: vec![(4, b"1.0.1".to_vec())],
            },
            TestPackage {
                title_id: 0x0100_0000_0001_1001,
                version: 0,
                kind: 2,
                flags: 0,
                files: vec![TestFile { path: "romfs/dlc.bin", data: b"dlc", zstd: false }],
                meta: vec![],
            },
        ])
    }

    pub(crate) fn write_temp(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nscb-nup-{}-{name}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{name}.nup"));
        File::create(&path).unwrap().write_all(bytes).unwrap();
        path
    }

    #[test]
    fn groups_digits() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(186_592), "186,592");
        assert_eq!(group_digits(1_234_567), "1,234,567");
    }

    #[test]
    fn reads_packages_tree_and_meta() {
        let path = write_temp("read", &sample());
        let nup = Nup::open(&path).unwrap();
        assert_eq!(nup.packages.len(), 3);
        let base = &nup.packages[0];
        assert_eq!(base.kind, PackageKind::Base);
        assert_eq!(base.label(), "base v0 [0100000000010000]");
        let files: Vec<String> = nup.walk(base.root).unwrap().into_iter().map(|(p, _)| p).collect();
        // `.listing` is a dump artifact and left out.
        assert_eq!(files, ["exefs/main", "exefs/main.npdm", "romfs/data/big.bin", "romfs/empty"]);
        let meta = nup.meta(base).unwrap();
        assert_eq!(meta.title_name.as_deref(), Some("Sample Game"));
        assert_eq!(meta.publisher.as_deref(), Some("Studio"));
        assert_eq!(nup.build_id(base, &meta), Some([0xAB; 32]));
        assert_eq!(nup.packages[2].kind, PackageKind::Addon);
        assert!(nup.packages[2].exefs.is_none());
        assert!(is_nup(&path));
    }

    #[test]
    fn hashes_match_and_corruption_is_found() {
        let bytes = sample();
        let path = write_temp("hash", &bytes);
        let nup = Nup::open(&path).unwrap();
        let mut file = File::open(&path).unwrap();
        let mut read = 0u64;
        for f in 0..nup.files.len() as u32 {
            assert_eq!(nup.hash_file(&mut file, f, &mut |n| read += n).unwrap(), None, "{}", nup.file_name(f));
        }
        assert_eq!(read, nup.files.iter().map(|f| f.extent_size).sum::<u64>());

        // Flip a byte inside the compressed big.bin extent.
        let (big, rec) = (0..nup.files.len() as u32).map(|f| (f, nup.files[f as usize])).find(|(f, _)| nup.file_name(*f) == "big.bin").unwrap();
        let mut damaged = bytes.clone();
        damaged[(rec.extent_offset + rec.extent_size / 2) as usize] ^= 0xFF;
        let path = write_temp("hash-damaged", &damaged);
        let nup = Nup::open(&path).unwrap();
        let mut file = File::open(&path).unwrap();
        assert!(nup.hash_file(&mut file, big, &mut |_| {}).unwrap().is_some());
    }

    #[test]
    fn rejects_damaged_toc_and_truncation() {
        let bytes = sample();
        let mut toc_damaged = bytes.clone();
        toc_damaged[0x80 + 0x40] ^= 1;
        let err = Nup::open(&write_temp("toc", &toc_damaged)).unwrap_err().to_string();
        assert!(err.contains("SHA-256"), "{err}");
        let err = Nup::open(&write_temp("short", &bytes[..bytes.len() - 1])).unwrap_err().to_string();
        assert!(err.contains("header says"), "{err}");
        let err = Nup::open(&write_temp("magic", b"NOPE")).unwrap_err().to_string();
        assert!(err.contains("not a NUP"), "{err}");
    }
}
