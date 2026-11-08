//! UI integration tests: the real app window, rendered headlessly, driving a fake
//! nscb_rust backend script.
#![cfg(unix)]

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    px, size, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowHandle, WindowOptions,
};
use nscb_desktop::core::batch::GameStatus;
use nscb_desktop::core::tools::{nscb_binary_name, Tools};
use nscb_desktop::ui::app::{NscbApp, PageId};
use nscb_desktop::ui::pages::batch_merge::BatchMergePage;
use nscb_desktop::ui::pages::operation::OperationPage;
use nscb_desktop::ui::widgets::FileTarget;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const FAKE_BACKEND: &str = r#"#!/bin/sh
echo "[ARGS] $*"
out=""; prev=""
for a in "$@"; do [ "$prev" = "-o" ] && out="$a"; prev="$a"; done
case "$*" in *slow*) sleep 5;; esac
case "$*" in "-d "*) [ -n "$out" ] && touch "$out/Merged [0100000000010000] [v0] (1G).xci";; esac
printf 'Working [##] 2/4\r' >&2
echo "Done!"
case "$*" in *fail*) echo "error: simulated failure" >&2; exit 3;; esac
exit 0
"#;

struct Fixture {
    root: PathBuf,
    tools: Tools,
}

impl Fixture {
    fn new(name: &str, with_keys: bool) -> Self {
        let root = std::env::temp_dir().join(format!("nscb-ui-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let tools = Tools::at(root.join("tools")).unwrap();
        let backend = tools.dir().join(nscb_binary_name());
        fs::write(&backend, FAKE_BACKEND).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&backend, fs::Permissions::from_mode(0o755)).unwrap();
        if with_keys {
            fs::write(tools.keys_path(), "fake").unwrap();
        }
        Self { root, tools }
    }

    fn file(&self, rel: &str) -> String {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "x").unwrap();
        path.to_string_lossy().into_owned()
    }

    fn dir(&self, rel: &str) -> String {
        let path = self.root.join(rel);
        fs::create_dir_all(&path).unwrap();
        path.to_string_lossy().into_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn open_app(cx: &mut TestAppContext, tools: Tools) -> (WindowHandle<gpui_kit::base::Root>, Entity<NscbApp>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        nscb_desktop::ui::theme::apply(cx);
        let bounds = Bounds { origin: Point::default(), size: size(px(1100.), px(2000.)) };
        let (window, app) = gpui_kit::open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
            cx,
            |window, cx| cx.new(|cx| NscbApp::with_update_check(tools, false, window, cx)),
        )
        .unwrap();
        (window.downcast().unwrap(), app)
    })
}

fn page<T: 'static>(cx: &mut TestAppContext, app: &Entity<NscbApp>, id: PageId) -> Entity<T> {
    cx.update(|cx| app.read(cx).page(id).unwrap().downcast::<T>().unwrap())
}

/// Poll in real time; the backend is a real child process.
fn wait_until(cx: &mut TestAppContext, what: &str, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn render(cx: &mut TestAppContext, window: WindowHandle<gpui_kit::base::Root>) {
    cx.update_window(window.into(), |_, window, cx| window.render_frame(cx)).unwrap();
}

#[gpui_kit::test]
fn setup_screen_is_shown_until_keys_exist(cx: &mut TestAppContext) {
    let fixture = Fixture::new("setup", false);
    let (window, _) = open_app(cx, fixture.tools.clone());
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("import-keys").is_some());
        assert!(window.try_find("nav-Compress").is_none());
    })
    .unwrap();

    let fixture = Fixture::new("setup-keys", true);
    let (window, _) = open_app(cx, fixture.tools.clone());
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("import-keys").is_none());
        assert!(window.try_find("nav-Compress").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn navigation_switches_pages_and_keeps_state(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new("nav", true);
    let (window, app) = open_app(cx, fixture.tools.clone());
    let compress: Entity<OperationPage> = page(cx, &app, PageId::Compress);
    let file = fixture.file("games/a.nsp");
    cx.update_window(window.into(), |_, window, cx| {
        compress.update(cx, |p, cx| p.add_files(vec![file.clone()], window, cx));
        window.render_frame(cx);
        window.click("nav-Settings", cx);
        window.render_frame(cx);
        assert!(window.try_find("check-update").is_some());
        assert!(window.try_find("drop-zone").is_none());
        window.click("nav-Compress", cx);
        window.render_frame(cx);
        assert!(window.try_find("start").is_some(), "files survive navigation");
    })
    .unwrap();
    assert_eq!(cx.update(|cx| app.read(cx).active_page()), PageId::Compress);
}

#[gpui_kit::test]
fn compress_runs_each_file_and_reports_completion(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new("compress", true);
    let (window, app) = open_app(cx, fixture.tools.clone());
    let compress: Entity<OperationPage> = page(cx, &app, PageId::Compress);
    let files = vec![fixture.file("games/a.nsp"), fixture.file("games/b.xci")];

    cx.update_window(window.into(), |_, window, cx| {
        // Drops are filtered by extension; duplicates are ignored.
        compress.update(cx, |p, cx| p.add_files(files.clone(), window, cx));
        compress.update(cx, |p, cx| p.add_files(vec![files[0].clone()], window, cx));
        window.render_frame(cx);
        window.click("start", cx);
    })
    .unwrap();
    assert_eq!(cx.update(|cx| compress.read(cx).files().len()), 2);

    wait_until(cx, "compress to finish", |cx| cx.update(|cx| !compress.read(cx).run_view().running));
    render(cx, window);
    cx.update(|cx| {
        let run = compress.read(cx).run_view();
        assert_eq!(run.message, "Completed successfully!");
        assert_eq!(run.percent, 100.0);
        assert!(run.lines.iter().any(|l| l == "[File 1/2] a.nsp"), "{:?}", run.lines);
        assert!(run.lines.iter().any(|l| l == "[File 2/2] b.xci"), "{:?}", run.lines);
        let invocation = run.lines.iter().find(|l| l.starts_with("> nscb_rust -z")).unwrap();
        assert!(invocation.contains("--level 18"), "{invocation}");
        assert!(invocation.contains("--keys"), "{invocation}");
        assert!(run.lines.iter().any(|l| l == "Done!"));
    });
}

#[gpui_kit::test]
fn failed_run_reports_exit_code(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new("fail", true);
    let (window, app) = open_app(cx, fixture.tools.clone());
    let compress: Entity<OperationPage> = page(cx, &app, PageId::Compress);
    let file = fixture.file("games/fail.nsp");
    cx.update_window(window.into(), |_, window, cx| {
        compress.update(cx, |p, cx| p.add_files(vec![file.clone()], window, cx));
        window.render_frame(cx);
        window.click("start", cx);
    })
    .unwrap();
    wait_until(cx, "failure", |cx| cx.update(|cx| !compress.read(cx).run_view().running));
    cx.update(|cx| {
        let run = compress.read(cx).run_view();
        assert_eq!(run.message, "Completed with exit code: 3");
        assert!(run.lines.iter().any(|l| l == "ERROR: simulated failure"), "{:?}", run.lines);
        assert!(
            run.lines.iter().any(|l| l.starts_with("ERROR: nscb_rust exited with code 3")),
            "{:?}",
            run.lines
        );
    });
}

#[gpui_kit::test]
fn cancel_stops_the_run_without_completing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new("cancel", true);
    let (window, app) = open_app(cx, fixture.tools.clone());
    let compress: Entity<OperationPage> = page(cx, &app, PageId::Compress);
    let file = fixture.file("games/slow.nsp");
    cx.update_window(window.into(), |_, window, cx| {
        compress.update(cx, |p, cx| p.add_files(vec![file.clone()], window, cx));
        window.render_frame(cx);
        window.click("start", cx);
    })
    .unwrap();
    wait_until(cx, "backend output", |cx| {
        cx.update(|cx| compress.read(cx).run_view().lines.iter().any(|l| l.starts_with("[ARGS]")))
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("cancel", cx);
    })
    .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    cx.run_until_parked();
    cx.update(|cx| {
        let run = compress.read(cx).run_view();
        assert!(!run.running);
        assert_eq!(run.message, "Cancelled");
        assert!(!run.lines.iter().any(|l| l == "Done!"));
    });

    // The runner accepts a new run right away.
    let quick = fixture.file("games/quick.nsp");
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("clear", cx);
        compress.update(cx, |p, cx| p.add_files(vec![quick.clone()], window, cx));
        window.render_frame(cx);
        window.click("start", cx);
    })
    .unwrap();
    wait_until(cx, "second run", |cx| cx.update(|cx| !compress.read(cx).run_view().running));
    cx.update(|cx| assert_eq!(compress.read(cx).run_view().message, "Completed successfully!"));
}

#[gpui_kit::test]
fn batch_merge_skips_merged_games_and_merges_the_rest(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let fixture = Fixture::new("batch", true);
    fixture.file("base/Zelda/Zelda [01007EF00011E000][v0].nsp");
    fixture.file("base/Mario/Mario [0100000000010000][v0].nsp");
    fixture.file("out/Zelda [01007EF00011E000] [v0] (1G).xci");
    let base = fixture.dir("base");
    let out = fixture.dir("out");

    let (window, app) = open_app(cx, fixture.tools.clone());
    let batch: Entity<BatchMergePage> = page(cx, &app, PageId::BatchMerge);
    cx.update_window(window.into(), |_, window, cx| {
        window.click("nav-Batch Merge", cx);
        batch.update(cx, |p, cx| p.set_folders(&base, &out, window, cx));
        window.render_frame(cx);
        window.click("scan", cx);
    })
    .unwrap();
    wait_until(cx, "scan", |cx| cx.update(|cx| !batch.read(cx).is_scanning() && !batch.read(cx).games().is_empty()));

    cx.update(|cx| {
        let games = batch.read(cx).games();
        assert_eq!(games.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Mario", "Zelda"]);
        assert_eq!(games[0].status, GameStatus::Pending);
        assert_eq!(games[1].status, GameStatus::Skipped);
    });

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("start", cx);
    })
    .unwrap();
    wait_until(cx, "batch merge", |cx| cx.update(|cx| !batch.read(cx).is_running()));
    cx.update(|cx| {
        let page = batch.read(cx);
        let mario = &page.games()[0];
        assert_eq!(mario.status, GameStatus::Done, "{:?}", page.run_view().lines);
        assert_eq!(mario.output_file.as_deref(), Some("Merged [0100000000010000] [v0] (1G).xci"));
        assert_eq!(page.games()[1].status, GameStatus::Skipped);
        assert_eq!(page.run_view().message, "Batch merge complete!");
        let invocation = page.run_view().lines.iter().find(|l| l.starts_with("> nscb_rust -d")).unwrap();
        assert!(invocation.contains("-t xci"), "{invocation}");
    });
}
