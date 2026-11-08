//! Application icons (the original NSCB Desktop SVGs) layered over gpui-kit's assets.

use gpui_kit::component::Icon;
use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

macro_rules! app_icons {
    ($($variant:ident => $file:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum AppIcon { $($variant),* }

        impl AppIcon {
            pub fn path(self) -> &'static str {
                match self { $(AppIcon::$variant => concat!("nscb/", $file, ".svg")),* }
            }
        }

        const ICON_FILES: &[(&str, &[u8])] = &[
            $((concat!("nscb/", $file, ".svg"), include_bytes!(concat!("../../assets/icons/", $file, ".svg")))),*
        ];
    };
}

app_icons! {
    Compress => "compress",
    Decompress => "decompress",
    Merge => "merge",
    Convert => "convert",
    Split => "split",
    Dspl => "dspl",
    Create => "create",
    Rename => "rename",
    Settings => "settings",
    Upload => "upload",
    File => "file",
    Folder => "folder",
    X => "x",
    Check => "check",
    AlertCircle => "alert-circle",
    Play => "play",
    Key => "key",
    Trash => "trash",
    Stop => "stop",
    SwitchLogo => "switch-logo",
    Info => "info",
    BatchMerge => "batch-merge",
    Verify => "verify",
}

impl AppIcon {
    pub fn icon(self) -> Icon {
        Icon::empty().path(self.path())
    }
}

impl From<AppIcon> for Icon {
    fn from(icon: AppIcon) -> Self {
        icon.icon()
    }
}

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = ICON_FILES.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(
            ICON_FILES
                .iter()
                .filter(|(p, _)| p.starts_with(path))
                .map(|(p, _)| SharedString::from(*p)),
        );
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}
