//! Command-line construction for the nscb_rust backend.

/// A backend operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    Compress,
    Decompress,
    Merge,
    Convert,
    Split,
    Dspl,
    Info,
    Create,
    Renamef,
    NutdbRefresh,
    NutdbLookup,
    Verify,
}

impl Op {
    pub fn name(self) -> &'static str {
        match self {
            Op::Compress => "compress",
            Op::Decompress => "decompress",
            Op::Merge => "merge",
            Op::Convert => "convert",
            Op::Split => "split",
            Op::Dspl => "dspl",
            Op::Info => "info",
            Op::Create => "create",
            Op::Renamef => "renamef",
            Op::NutdbRefresh => "nutdb-refresh",
            Op::NutdbLookup => "nutdb-lookup",
            Op::Verify => "verify",
        }
    }

    /// Ops that accept one input per invocation; the runner batches them per file.
    pub fn is_single_file(self) -> bool {
        matches!(
            self,
            Op::Compress
                | Op::Decompress
                | Op::Convert
                | Op::Split
                | Op::Dspl
                | Op::Info
                | Op::Renamef
                | Op::Verify
        )
    }

    /// NUTDB ops never receive `--keys`.
    pub fn uses_keys(self) -> bool {
        !matches!(self, Op::NutdbRefresh | Op::NutdbLookup)
    }

    fn takes_output_flag(self) -> bool {
        !matches!(
            self,
            Op::Create | Op::Info | Op::Renamef | Op::NutdbRefresh | Op::NutdbLookup | Op::Verify
        )
    }
}

/// Options for a run. Empty strings are treated as unset.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunOptions {
    pub output: Option<String>,
    pub level: Option<u32>,
    pub format: Option<String>,
    pub nodelta: bool,
    pub pv: bool,
    pub rsvcap: Option<String>,
    pub keypatch: Option<String>,
    /// Info mode: `filelist` or `contentlist` (default).
    pub mode: Option<String>,
    pub renmode: Option<String>,
    pub addlangue: Option<String>,
    pub noversion: Option<String>,
    pub dlcrname: Option<String>,
    pub nutdb_url: Option<String>,
    pub title_id: Option<String>,
    pub vertype: Option<String>,
    pub filelist_path: Option<String>,
    pub buffer: Option<String>,
    pub temp_dir: Option<String>,
}

fn set(v: &Option<String>) -> Option<&str> {
    v.as_deref().filter(|s| !s.is_empty())
}

/// Last path component, accepting both `/` and `\` separators.
pub fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Parent directory with separators normalised to `/`; `.` for a bare file name.
pub fn dirname(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    match normalized.rfind('/') {
        Some(i) => normalized[..i].to_string(),
        None => ".".to_string(),
    }
}

pub fn build_args(op: Op, files: &[String], opts: &RunOptions, keys_path: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut push = |a: &str| args.push(a.to_string());

    match op {
        Op::Compress => {
            push("-z");
            files.iter().for_each(|f| push(f));
            if let Some(level) = opts.level {
                push("--level");
                push(&level.to_string());
            }
        }
        Op::Decompress => {
            push("--decompress");
            files.iter().for_each(|f| push(f));
        }
        Op::Merge => {
            push("-d");
            files.iter().for_each(|f| push(f));
            if let Some(f) = set(&opts.format) {
                push("-t");
                push(f);
            }
            if opts.nodelta {
                push("-n");
            }
            if opts.pv {
                push("--pv");
            }
            if let Some(v) = set(&opts.rsvcap) {
                push("--RSVcap");
                push(v);
            }
            if let Some(v) = set(&opts.keypatch) {
                push("-k");
                push(v);
            }
        }
        Op::Convert => {
            push("-c");
            files.iter().for_each(|f| push(f));
            if let Some(f) = set(&opts.format) {
                push("-t");
                push(f);
            }
        }
        Op::Split => {
            push("--splitter");
            files.iter().for_each(|f| push(f));
        }
        Op::Dspl => {
            push("--dspl");
            files.iter().for_each(|f| push(f));
            if let Some(f) = set(&opts.format) {
                push("-t");
                push(f);
            }
        }
        Op::Info => {
            if set(&opts.mode) == Some("filelist") {
                push("--ADVfilelist");
            } else {
                push("--ADVcontentlist");
            }
            files.iter().for_each(|f| push(f));
        }
        Op::Create => {
            let folder = files.first().map(String::as_str).unwrap_or_default();
            let out_dir = set(&opts.output).map(str::to_string).unwrap_or_else(|| dirname(folder));
            push("--create");
            push(&format!("{out_dir}/{}.nsp", basename(folder)));
            push("--ifolder");
            push(folder);
        }
        Op::Renamef => {
            push("--renamef");
            push(files.first().map(String::as_str).unwrap_or_default());
            for (flag, value) in [
                ("--renmode", &opts.renmode),
                ("--addlangue", &opts.addlangue),
                ("--noversion", &opts.noversion),
                ("--dlcrname", &opts.dlcrname),
            ] {
                if let Some(v) = set(value) {
                    push(flag);
                    push(v);
                }
            }
        }
        Op::NutdbRefresh => {
            push("--nutdb-refresh");
            if let Some(url) = set(&opts.nutdb_url) {
                push("--nutdb-url");
                push(url);
            }
        }
        Op::NutdbLookup => {
            push("--nutdb-lookup");
            push(opts.title_id.as_deref().unwrap_or_default());
        }
        Op::Verify => {
            if let Some(list) = set(&opts.filelist_path) {
                push("--verify");
                push("all");
                push("--text_file");
                push(list);
            } else {
                push("--verify");
                files.iter().for_each(|f| push(f));
            }
            if let Some(v) = set(&opts.vertype) {
                push("--vertype");
                push(v);
            }
        }
    }

    if op.takes_output_flag() {
        if let Some(out) = set(&opts.output) {
            push("-o");
            push(out);
        } else if let Some(first) = files.first() {
            push("-o");
            push(&dirname(first));
        }
    }

    if let Some(b) = set(&opts.buffer) {
        push("-b");
        push(b);
    }
    if let Some(t) = set(&opts.temp_dir) {
        push("--temp-dir");
        push(t);
    }
    if let Some(k) = keys_path {
        push("--keys");
        push(k);
    }

    args
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: Option<&str> = Some("C:/tools/prod.keys");

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn files(items: &[&str]) -> Vec<String> {
        v(items)
    }

    fn s(x: &str) -> Option<String> {
        Some(x.to_string())
    }

    fn after<'a>(args: &'a [String], flag: &str) -> &'a str {
        let i = args.iter().position(|a| a == flag).expect("flag present");
        &args[i + 1]
    }

    fn has(args: &[String], item: &str) -> bool {
        args.iter().any(|a| a == item)
    }

    // ---- helpers ----

    #[test]
    fn basename_unix_and_windows() {
        assert_eq!(basename("/home/user/game.nsp"), "game.nsp");
        assert_eq!(basename("C:\\Games\\game.xci"), "game.xci");
    }

    #[test]
    fn dirname_unix_windows_and_bare() {
        assert_eq!(dirname("/home/user/game.nsp"), "/home/user");
        assert_eq!(dirname("C:\\Games\\game.xci"), "C:/Games");
        assert_eq!(dirname("game.nsp"), ".");
    }

    // ---- single-file ops ----

    #[test]
    fn single_file_ops() {
        for op in [
            Op::Compress,
            Op::Decompress,
            Op::Convert,
            Op::Split,
            Op::Dspl,
            Op::Info,
            Op::Renamef,
            Op::Verify,
        ] {
            assert!(op.is_single_file(), "{op:?}");
        }
        for op in [Op::Merge, Op::Create, Op::NutdbRefresh, Op::NutdbLookup] {
            assert!(!op.is_single_file(), "{op:?}");
        }
    }

    // ---- temp dir ----

    #[test]
    fn temp_dir_passed_when_configured() {
        let opts = RunOptions { temp_dir: s("D:/NSCB Temp"), ..Default::default() };
        let args = build_args(Op::Compress, &files(&["H:/game.nsp"]), &opts, KEYS);
        assert_eq!(after(&args, "--temp-dir"), "D:/NSCB Temp");
    }

    #[test]
    fn temp_dir_omitted_when_empty() {
        let opts = RunOptions { temp_dir: s(""), ..Default::default() };
        let args = build_args(Op::Compress, &files(&["H:/game.nsp"]), &opts, KEYS);
        assert!(!has(&args, "--temp-dir"));
    }

    // ---- compress ----

    #[test]
    fn compress_defaults() {
        let args = build_args(Op::Compress, &files(&["H:/game.nsp"]), &Default::default(), KEYS);
        assert_eq!(args, v(&["-z", "H:/game.nsp", "-o", "H:", "--keys", "C:/tools/prod.keys"]));
    }

    #[test]
    fn compress_with_level() {
        let opts = RunOptions { level: Some(22), ..Default::default() };
        let args = build_args(Op::Compress, &files(&["H:/game.nsp"]), &opts, KEYS);
        assert_eq!(
            args,
            v(&["-z", "H:/game.nsp", "--level", "22", "-o", "H:", "--keys", "C:/tools/prod.keys"])
        );
    }

    #[test]
    fn compress_custom_output() {
        let opts = RunOptions { output: s("D:/out"), ..Default::default() };
        let args = build_args(Op::Compress, &files(&["H:/game.nsp"]), &opts, KEYS);
        assert_eq!(args, v(&["-z", "H:/game.nsp", "-o", "D:/out", "--keys", "C:/tools/prod.keys"]));
    }

    #[test]
    fn compress_with_buffer() {
        let opts = RunOptions { buffer: s("64"), ..Default::default() };
        let args = build_args(Op::Compress, &files(&["H:/game.nsp"]), &opts, KEYS);
        assert_eq!(
            args,
            v(&["-z", "H:/game.nsp", "-o", "H:", "-b", "64", "--keys", "C:/tools/prod.keys"])
        );
    }

    #[test]
    fn no_keys_when_keys_path_missing() {
        let args = build_args(Op::Compress, &files(&["H:/game.nsp"]), &Default::default(), None);
        assert!(!has(&args, "--keys"));
    }

    // ---- decompress ----

    #[test]
    fn decompress_single() {
        let args = build_args(Op::Decompress, &files(&["H:/game.nsz"]), &Default::default(), KEYS);
        assert_eq!(
            args,
            v(&["--decompress", "H:/game.nsz", "-o", "H:", "--keys", "C:/tools/prod.keys"])
        );
    }

    #[test]
    fn decompress_custom_output() {
        let opts = RunOptions { output: s("D:/out"), ..Default::default() };
        let args = build_args(Op::Decompress, &files(&["H:/game.nsz"]), &opts, KEYS);
        assert_eq!(
            args,
            v(&["--decompress", "H:/game.nsz", "-o", "D:/out", "--keys", "C:/tools/prod.keys"])
        );
    }

    // ---- merge ----

    #[test]
    fn merge_two_files_with_format() {
        let opts = RunOptions { format: s("xci"), ..Default::default() };
        let args = build_args(Op::Merge, &files(&["H:/base.nsp", "H:/update.nsp"]), &opts, KEYS);
        assert_eq!(
            args,
            v(&[
                "-d",
                "H:/base.nsp",
                "H:/update.nsp",
                "-t",
                "xci",
                "-o",
                "H:",
                "--keys",
                "C:/tools/prod.keys"
            ])
        );
    }

    #[test]
    fn merge_flags() {
        let f = files(&["H:/a.nsp", "H:/b.nsp"]);
        let opts = RunOptions { format: s("nsp"), nodelta: true, ..Default::default() };
        assert!(has(&build_args(Op::Merge, &f, &opts, KEYS), "-n"));

        let opts = RunOptions { pv: true, ..Default::default() };
        assert!(has(&build_args(Op::Merge, &f, &opts, KEYS), "--pv"));

        let opts = RunOptions { rsvcap: s("5"), ..Default::default() };
        assert_eq!(after(&build_args(Op::Merge, &f, &opts, KEYS), "--RSVcap"), "5");

        let opts = RunOptions { keypatch: s("3"), ..Default::default() };
        assert_eq!(after(&build_args(Op::Merge, &f, &opts, KEYS), "-k"), "3");

        let opts = RunOptions { output: s("D:/merged"), ..Default::default() };
        assert_eq!(after(&build_args(Op::Merge, &f, &opts, KEYS), "-o"), "D:/merged");
    }

    #[test]
    fn merge_passes_all_files_in_one_invocation() {
        let args = build_args(
            Op::Merge,
            &files(&["H:/a.nsp", "H:/b.nsp", "H:/c.nsp"]),
            &Default::default(),
            KEYS,
        );
        assert_eq!(&args[..4], &v(&["-d", "H:/a.nsp", "H:/b.nsp", "H:/c.nsp"])[..]);
    }

    // ---- convert / split / dspl ----

    #[test]
    fn convert_with_format() {
        let opts = RunOptions { format: s("xci"), ..Default::default() };
        let args = build_args(Op::Convert, &files(&["H:/game.nsp"]), &opts, KEYS);
        assert_eq!(
            args,
            v(&["-c", "H:/game.nsp", "-t", "xci", "-o", "H:", "--keys", "C:/tools/prod.keys"])
        );
        let opts = RunOptions { format: s("nsp"), ..Default::default() };
        let args = build_args(Op::Convert, &files(&["H:/game.xci"]), &opts, KEYS);
        assert_eq!(after(&args, "-t"), "nsp");
    }

    #[test]
    fn split_single() {
        let args = build_args(Op::Split, &files(&["H:/game.xci"]), &Default::default(), KEYS);
        assert_eq!(
            args,
            v(&["--splitter", "H:/game.xci", "-o", "H:", "--keys", "C:/tools/prod.keys"])
        );
    }

    #[test]
    fn dspl_with_format() {
        let opts = RunOptions { format: s("xci"), ..Default::default() };
        let args = build_args(Op::Dspl, &files(&["H:/game.xci"]), &opts, KEYS);
        assert_eq!(
            args,
            v(&["--dspl", "H:/game.xci", "-t", "xci", "-o", "H:", "--keys", "C:/tools/prod.keys"])
        );
        let opts = RunOptions { format: s("nsp"), ..Default::default() };
        assert_eq!(after(&build_args(Op::Dspl, &files(&["H:/game.xci"]), &opts, KEYS), "-t"), "nsp");
    }

    // ---- info ----

    #[test]
    fn info_modes_and_no_output_flag() {
        let args = build_args(Op::Info, &files(&["H:/game.nsp"]), &Default::default(), KEYS);
        assert_eq!(args, v(&["--ADVcontentlist", "H:/game.nsp", "--keys", "C:/tools/prod.keys"]));
        assert!(!has(&args, "-o"));
        let opts = RunOptions { mode: s("filelist"), ..Default::default() };
        let args = build_args(Op::Info, &files(&["H:/game.nsp"]), &opts, KEYS);
        assert_eq!(args, v(&["--ADVfilelist", "H:/game.nsp", "--keys", "C:/tools/prod.keys"]));
    }

    // ---- create ----

    #[test]
    fn create_from_folder() {
        let args = build_args(Op::Create, &files(&["H:/MyGame"]), &Default::default(), KEYS);
        assert_eq!(
            args,
            v(&["--create", "H:/MyGame.nsp", "--ifolder", "H:/MyGame", "--keys", "C:/tools/prod.keys"])
        );
        assert!(!has(&args, "-o"));
    }

    #[test]
    fn create_custom_output() {
        let opts = RunOptions { output: s("D:/out"), ..Default::default() };
        let args = build_args(Op::Create, &files(&["H:/MyGame"]), &opts, KEYS);
        assert_eq!(
            args,
            v(&["--create", "D:/out/MyGame.nsp", "--ifolder", "H:/MyGame", "--keys", "C:/tools/prod.keys"])
        );
    }

    #[test]
    fn create_windows_path() {
        let args =
            build_args(Op::Create, &files(&["C:\\Users\\me\\GameFolder"]), &Default::default(), KEYS);
        assert_eq!(after(&args, "--create"), "C:/Users/me/GameFolder.nsp");
        assert_eq!(after(&args, "--ifolder"), "C:\\Users\\me\\GameFolder");
    }

    // ---- rename ----

    #[test]
    fn renamef_all_options() {
        let opts = RunOptions {
            renmode: s("force"),
            addlangue: s("true"),
            noversion: s("false"),
            dlcrname: s("tag"),
            ..Default::default()
        };
        let args = build_args(Op::Renamef, &files(&["H:/game.nsp"]), &opts, KEYS);
        assert_eq!(
            args,
            v(&[
                "--renamef",
                "H:/game.nsp",
                "--renmode",
                "force",
                "--addlangue",
                "true",
                "--noversion",
                "false",
                "--dlcrname",
                "tag",
                "--keys",
                "C:/tools/prod.keys"
            ])
        );
    }

    #[test]
    fn renamef_folder_and_first_file_only() {
        let opts = RunOptions { renmode: s("force"), ..Default::default() };
        let args = build_args(Op::Renamef, &files(&["H:/Games"]), &opts, KEYS);
        assert_eq!(
            args,
            v(&["--renamef", "H:/Games", "--renmode", "force", "--keys", "C:/tools/prod.keys"])
        );
        let args = build_args(Op::Renamef, &files(&["H:/a.nsp", "H:/b.nsp"]), &opts, KEYS);
        assert_eq!(args.iter().filter(|a| *a == "H:/a.nsp").count(), 1);
        assert!(!has(&args, "H:/b.nsp"));
        assert!(!has(&args, "-o"));
    }

    #[test]
    fn renamef_modes_and_omitted_flags() {
        for mode in ["skip_corr_tid", "skip_if_tid"] {
            let opts = RunOptions { renmode: s(mode), ..Default::default() };
            let args = build_args(Op::Renamef, &files(&["H:/game.nsp"]), &opts, KEYS);
            assert_eq!(after(&args, "--renmode"), mode);
        }
        let args = build_args(Op::Renamef, &files(&["H:/game.nsp"]), &Default::default(), KEYS);
        for flag in ["--renmode", "--addlangue", "--noversion", "--dlcrname"] {
            assert!(!has(&args, flag), "{flag}");
        }
    }

    // ---- NUTDB ----

    #[test]
    fn nutdb_refresh() {
        assert_eq!(build_args(Op::NutdbRefresh, &[], &Default::default(), None), v(&["--nutdb-refresh"]));
        // The runner gates keys for NUTDB ops; build_args appends whatever it is given.
        assert!(has(&build_args(Op::NutdbRefresh, &[], &Default::default(), KEYS), "--keys"));
        assert!(!Op::NutdbRefresh.uses_keys());
        let url = "https://gh-proxy.org/https://raw.githubusercontent.com/blawar/titledb/master/US.en.json";
        let opts = RunOptions { nutdb_url: s(url), ..Default::default() };
        assert_eq!(
            build_args(Op::NutdbRefresh, &[], &opts, None),
            v(&["--nutdb-refresh", "--nutdb-url", url])
        );
        let opts = RunOptions { nutdb_url: s(""), ..Default::default() };
        assert_eq!(build_args(Op::NutdbRefresh, &[], &opts, None), v(&["--nutdb-refresh"]));
    }

    #[test]
    fn nutdb_lookup() {
        let opts = RunOptions { title_id: s("0100000000010000"), ..Default::default() };
        assert_eq!(
            build_args(Op::NutdbLookup, &[], &opts, None),
            v(&["--nutdb-lookup", "0100000000010000"])
        );
    }

    // ---- verify ----

    #[test]
    fn verify_filelist_mode() {
        for level in ["lv1", "lv2", "lv3"] {
            let opts = RunOptions {
                vertype: s(level),
                filelist_path: s("C:/tools/game-vflist.txt"),
                ..Default::default()
            };
            let args = build_args(Op::Verify, &files(&["H:/game.nsp"]), &opts, KEYS);
            assert_eq!(
                args,
                v(&[
                    "--verify",
                    "all",
                    "--text_file",
                    "C:/tools/game-vflist.txt",
                    "--vertype",
                    level,
                    "--keys",
                    "C:/tools/prod.keys"
                ])
            );
            assert!(!has(&args, "-o"));
        }
    }
}
