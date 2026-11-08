//! Batch merge discovery: each subfolder of a base folder is one game; existing merged
//! outputs are matched by title ID (or folder name) so already-merged games are skipped.

use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

pub const SWITCH_EXTS: &[&str] = &["nsp", "xci", "nsz", "xcz", "ncz"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameStatus {
    Pending,
    Skipped,
    Running,
    Done,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatchGame {
    pub name: String,
    pub path: String,
    pub files: Vec<String>,
    pub status: GameStatus,
    pub message: Option<String>,
    pub output_file: Option<String>,
}

/// Parsed nscb_rust merge output name: `GameName [TitleID] [vVersion] (Summary).ext`,
/// e.g. `Bayonetta 2 [0100847008600000] [v131072] (1G+1U+3D).nsp`.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedMergeOutput {
    pub filename: String,
    pub game_name: String,
    /// Uppercase 16-hex title ID.
    pub title_id: String,
    pub version: u64,
    /// DLC packs included in the merge, from the summary.
    pub dlc_count: u64,
}

static OUTPUT_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(.+?)\s+\[([0-9A-F]{16})\]\s+\[v([0-9]+)\](?:\s+\(([^)]+)\))?$").unwrap()
});
static DLC_SUMMARY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)([0-9]+)D").unwrap());
static TITLE_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[\[-]([0-9A-F]{16})[\]-]").unwrap());
static VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\[v([0-9]+)\]").unwrap());

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_lowercase())
        .unwrap_or_default()
}

pub fn is_switch_file(name: &str) -> bool {
    SWITCH_EXTS.contains(&extension(name).as_str())
}

pub fn parse_merge_output_filename(filename: &str) -> Option<ParsedMergeOutput> {
    let (base, _) = filename.rsplit_once('.')?;
    let c = OUTPUT_NAME.captures(base)?;
    let dlc_count = c
        .get(4)
        .and_then(|s| DLC_SUMMARY.captures(s.as_str()))
        .and_then(|d| d[1].parse().ok())
        .unwrap_or(0);
    Some(ParsedMergeOutput {
        filename: filename.to_string(),
        game_name: c[1].to_string(),
        title_id: c[2].to_uppercase(),
        version: c[3].parse().unwrap_or(0),
        dlc_count,
    })
}

fn title_ids(paths: &[String]) -> impl Iterator<Item = String> + '_ {
    paths.iter().flat_map(|p| {
        TITLE_ID
            .captures_iter(file_name(p))
            .map(|c| c[1].to_uppercase())
            .collect::<Vec<_>>()
    })
}

/// All base title IDs referenced by the inputs. Update (`800`) and DLC (`001`+) IDs are
/// normalised to their base (`000`), so multi-game collection folders can match an
/// output named after any of the included games.
pub fn extract_base_title_ids(paths: &[String]) -> HashSet<String> {
    title_ids(paths)
        .map(|tid| {
            if tid.ends_with("000") {
                tid
            } else {
                format!("{}000", &tid[..tid.len() - 3])
            }
        })
        .collect()
}

/// Distinct DLC title IDs (suffix `001`–`7FF`) among the inputs.
pub fn count_dlc_files(paths: &[String]) -> u64 {
    title_ids(paths)
        .filter(|tid| {
            let suffix = u32::from_str_radix(&tid[tid.len() - 3..], 16).unwrap_or(0);
            (0x001..=0x7FF).contains(&suffix)
        })
        .collect::<HashSet<_>>()
        .len() as u64
}

/// Highest `[vN]` found among the input names.
pub fn extract_latest_version(paths: &[String]) -> u64 {
    paths
        .iter()
        .flat_map(|p| {
            VERSION
                .captures_iter(file_name(p))
                .filter_map(|c| c[1].parse::<u64>().ok())
                .collect::<Vec<_>>()
        })
        .max()
        .unwrap_or(0)
}

pub fn normalize_for_match(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect()
}

fn highest_version<'a>(candidates: impl Iterator<Item = &'a ParsedMergeOutput>) -> Option<&'a ParsedMergeOutput> {
    candidates.fold(None, |best: Option<&ParsedMergeOutput>, o| match best {
        Some(b) if o.version <= b.version => Some(b),
        _ => Some(o),
    })
}

/// Find an up-to-date merged output for a game folder, if any.
pub fn match_existing_output<'a>(
    folder_name: &str,
    files: &[String],
    outputs: &'a [ParsedMergeOutput],
) -> Option<&'a ParsedMergeOutput> {
    let input_version = extract_latest_version(files);
    let input_dlc = count_dlc_files(files);
    let is_stale = |o: &ParsedMergeOutput| input_version > o.version || input_dlc > o.dlc_count;

    // Title ID match: most accurate, handles collections. Highest version wins because
    // the output folder may contain both an old and a new merge of the same game.
    let base_tids = extract_base_title_ids(files);
    if !base_tids.is_empty() {
        if let Some(o) = highest_version(outputs.iter().filter(|o| base_tids.contains(&o.title_id))) {
            // A stale TID match (newer update or more DLC in the folder) means this game
            // needs merging; don't let the name fallback re-match the same output.
            return if is_stale(o) { None } else { Some(o) };
        }
    }

    // Name fallback for files without title IDs:
    //   ends_with   — publisher prefix: "Metal Slug 3" in "ACA NEOGEO METAL SLUG 3"
    //   starts_with — volume suffix: "Mega Man BN Legacy Collection" in "... Vol.1",
    //                 rejected when the extra part starts with a digit (a sequel).
    let norm = normalize_for_match(folder_name);
    if norm.len() < 3 {
        return None;
    }
    let candidate = highest_version(outputs.iter().filter(|o| {
        let n = normalize_for_match(&o.game_name);
        n == norm
            || n.ends_with(&norm)
            || (n.starts_with(&norm)
                && !n[norm.len()..].starts_with(|c: char| c.is_ascii_digit()))
    }))?;
    if is_stale(candidate) {
        None
    } else {
        Some(candidate)
    }
}

pub struct DirEntryInfo {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

pub fn list_dir(dir: &str) -> std::io::Result<Vec<DirEntryInfo>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        out.push(DirEntryInfo {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path().to_string_lossy().into_owned(),
            is_dir,
        });
    }
    Ok(out)
}

/// Discover games under `base_folder` and mark those already merged into `output_dir`.
pub fn scan(base_folder: &str, output_dir: &str) -> std::io::Result<Vec<BatchGame>> {
    let subfolders: Vec<_> = list_dir(base_folder)?.into_iter().filter(|e| e.is_dir).collect();

    let outputs: Vec<ParsedMergeOutput> = list_dir(output_dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|f| !f.is_dir && is_switch_file(&f.name))
        .filter_map(|f| parse_merge_output_filename(&f.name))
        .collect();

    let mut games = Vec::new();
    for sub in subfolders {
        if sub.name.is_empty() {
            continue;
        }
        let Ok(entries) = list_dir(&sub.path) else { continue };
        let files: Vec<String> = entries
            .into_iter()
            .filter(|e| !e.is_dir && is_switch_file(&e.name))
            .map(|e| e.path)
            .collect();
        if files.is_empty() {
            continue;
        }
        let matched = match_existing_output(&sub.name, &files, &outputs);
        games.push(BatchGame {
            status: if matched.is_some() { GameStatus::Skipped } else { GameStatus::Pending },
            output_file: matched.map(|o| o.filename.clone()),
            name: sub.name,
            path: sub.path,
            files,
            message: None,
        });
    }
    games.sort_by_key(|g| g.name.to_lowercase());
    Ok(games)
}

/// Names of regular files in `dir` (empty if unreadable).
pub fn file_names(dir: &str) -> HashSet<String> {
    list_dir(dir)
        .map(|entries| entries.into_iter().filter(|e| !e.is_dir).map(|e| e.name).collect())
        .unwrap_or_default()
}

/// Join with the separator style already used by `base`.
pub fn path_join(base: &str, name: &str) -> String {
    let sep = if base.contains('\\') { '\\' } else { '/' };
    format!("{}{sep}{name}", base.trim_end_matches(['/', '\\']))
}

/// After a failed merge, prefix new outputs with `ERROR_` so they aren't mistaken for good
/// merges on the next scan. Returns the name to show for the game.
pub fn mark_failed_outputs(output_dir: &str, new_files: &[String]) -> Option<String> {
    let mut shown = None;
    for name in new_files {
        if name.starts_with("ERROR_") {
            continue;
        }
        let renamed = format!("ERROR_{name}");
        let ok = fs::rename(path_join(output_dir, name), path_join(output_dir, &renamed)).is_ok();
        shown = Some(if ok { renamed } else { name.clone() });
    }
    shown
}

pub fn exists_dir(path: &str) -> bool {
    Path::new(path).is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(name: &str) -> ParsedMergeOutput {
        parse_merge_output_filename(name).unwrap()
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_output_name() {
        let p = out("Bayonetta 2 [0100847008600000] [v131072] (1G+1U+3D).nsp");
        assert_eq!(p.game_name, "Bayonetta 2");
        assert_eq!(p.title_id, "0100847008600000");
        assert_eq!(p.version, 131072);
        assert_eq!(p.dlc_count, 3);
        assert_eq!(out("Game [0100abcdef000000] [v0].xci").title_id, "0100ABCDEF000000");
        assert!(parse_merge_output_filename("random file.nsp").is_none());
    }

    #[test]
    fn base_ids_normalise_updates_and_dlc() {
        let ids = extract_base_title_ids(&v(&[
            "/g/Game [0100847008600800][v65536].nsp",
            "/g/Game DLC [0100847008600001].nsp",
            "C:\\g\\Other-0100AAAABBBB0000-.xci",
        ]));
        assert!(ids.contains("0100847008600000"));
        assert!(ids.contains("0100AAAABBBB0000"));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn dlc_count_and_latest_version() {
        let files = v(&[
            "Game [0100847008600000][v0].nsp",
            "Game [0100847008600800][v131072].nsp",
            "DLC [0100847008601001][v0].nsp",
            "DLC [0100847008601002][v0].nsp",
        ]);
        assert_eq!(count_dlc_files(&files), 2);
        assert_eq!(extract_latest_version(&files), 131072);
    }

    #[test]
    fn tid_match_skips_up_to_date_output() {
        let outputs = vec![
            out("Bayonetta 2 [0100847008600000] [v65536] (1G+1U).nsp"),
            out("Bayonetta 2 [0100847008600000] [v131072] (1G+1U+1D).nsp"),
        ];
        let files = v(&["Bayonetta 2 [0100847008600000][v0].nsp", "Upd [0100847008600800][v131072].nsp"]);
        let m = match_existing_output("Bayonetta 2", &files, &outputs).unwrap();
        assert_eq!(m.version, 131072);
    }

    #[test]
    fn stale_tid_match_is_not_rematched_by_name() {
        let outputs = vec![out("Bayonetta 2 [0100847008600000] [v0] (1G).nsp")];
        let files = v(&["Bayonetta 2 [0100847008600000][v0].nsp", "Upd [0100847008600800][v65536].nsp"]);
        assert!(match_existing_output("Bayonetta 2", &files, &outputs).is_none());
        // New DLC without a version bump is stale too.
        let files = v(&["Bayonetta 2 [0100847008600000][v0].nsp", "DLC [0100847008601001][v0].nsp"]);
        assert!(match_existing_output("Bayonetta 2", &files, &outputs).is_none());
    }

    #[test]
    fn name_fallback_handles_prefix_and_suffix_but_not_sequels() {
        let outputs = vec![
            out("ACA NEOGEO METAL SLUG 3 [0100AAAAAAAA0000] [v0].nsp"),
            out("MEGAMAN BN LEGACY COLLECTION Vol.1 [0100BBBBBBBB0000] [v0].nsp"),
            out("Bayonetta 2 [0100CCCCCCCC0000] [v0].nsp"),
        ];
        let no_tid = v(&["game.nsp"]);
        assert!(match_existing_output("Metal Slug 3", &no_tid, &outputs).is_some());
        assert!(match_existing_output("MegaMan BN Legacy Collection", &no_tid, &outputs).is_some());
        assert!(match_existing_output("Bayonetta", &no_tid, &outputs).is_none());
        assert!(match_existing_output("ab", &no_tid, &outputs).is_none());
    }

    #[test]
    fn scan_discovers_games_and_marks_merged_ones() {
        let root = std::env::temp_dir().join(format!("nscb-batch-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let base = root.join("base");
        let output = root.join("out");
        for (dir, file) in [
            ("zelda", "Zelda [01007EF00011E000][v0].nsp"),
            ("Animal", "Animal [01006F8002326000][v0].nsp"),
            ("empty", "readme.txt"),
        ] {
            fs::create_dir_all(base.join(dir)).unwrap();
            fs::write(base.join(dir).join(file), "x").unwrap();
        }
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("Zelda [01007EF00011E000] [v0] (1G).nsp"), "x").unwrap();

        let games = scan(base.to_str().unwrap(), output.to_str().unwrap()).unwrap();
        assert_eq!(games.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Animal", "zelda"]);
        assert_eq!(games[0].status, GameStatus::Pending);
        assert_eq!(games[1].status, GameStatus::Skipped);
        assert_eq!(games[1].output_file.as_deref(), Some("Zelda [01007EF00011E000] [v0] (1G).nsp"));

        let renamed = mark_failed_outputs(output.to_str().unwrap(), &v(&["Zelda [01007EF00011E000] [v0] (1G).nsp"]));
        assert_eq!(renamed.as_deref(), Some("ERROR_Zelda [01007EF00011E000] [v0] (1G).nsp"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn path_join_keeps_separator_style() {
        assert_eq!(path_join("C:\\out\\", "a.nsp"), "C:\\out\\a.nsp");
        assert_eq!(path_join("/out", "a.nsp"), "/out/a.nsp");
    }
}
