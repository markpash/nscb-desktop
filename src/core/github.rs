//! GitHub release lookups for the nscb_rust backend and for app updates.
//! These calls block; run them on a background executor.

use serde::Deserialize;

const BACKEND_RELEASES: &str = "https://api.github.com/repos/cxfcxf/nscb_rust/releases/latest";
const APP_RELEASES: &str = "https://api.github.com/repos/cxfcxf/nscb-desktop/releases/latest";
const USER_AGENT: &str = concat!("nscb-desktop/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub tag: String,
    /// Asset download URL for the backend; release page URL for the app.
    pub url: String,
}

#[derive(Deserialize)]
struct ApiRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    #[serde(default)]
    name: String,
    #[serde(default)]
    browser_download_url: String,
}

fn fetch_release(url: &str) -> Option<ApiRelease> {
    ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?
        .body_mut()
        .read_json::<ApiRelease>()
        .ok()
}

/// Pick the backend asset for a platform (`std::env::consts::OS`).
fn pick_backend_asset<'a>(platform: &str, assets: &'a [ApiAsset]) -> Option<&'a ApiAsset> {
    assets.iter().find(|a| match platform {
        "macos" => a.name.contains("macos"),
        "linux" => a.name.contains("linux"),
        _ => a.name.ends_with(".exe"),
    })
}

pub fn latest_backend_release() -> Option<Release> {
    let release = fetch_release(BACKEND_RELEASES)?;
    let asset = pick_backend_asset(std::env::consts::OS, &release.assets)?;
    if asset.browser_download_url.is_empty() {
        return None;
    }
    Some(Release {
        tag: release.tag_name,
        url: asset.browser_download_url.clone(),
    })
}

pub fn latest_app_release() -> Option<Release> {
    let release = fetch_release(APP_RELEASES)?;
    if release.tag_name.is_empty() {
        return None;
    }
    Some(Release {
        tag: release.tag_name,
        url: release.html_url,
    })
}

/// Whether a release tag (`v1.2.3` or `1.2.3`) differs from the running version.
pub fn is_newer_tag(tag: &str, current: &str) -> bool {
    tag.trim_start_matches('v') != current
}

pub fn download(url: &str) -> Result<Vec<u8>, String> {
    let mut response = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("Download failed: {e}"))?;
    response
        .body_mut()
        .with_config()
        .limit(1024 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| format!("Failed to read response body: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> ApiAsset {
        ApiAsset {
            name: name.into(),
            browser_download_url: format!("https://example.invalid/{name}"),
        }
    }

    #[test]
    fn picks_platform_asset() {
        let assets = vec![
            asset("nscb_rust-linux-amd64"),
            asset("nscb_rust-macos-arm64"),
            asset("nscb_rust.exe"),
        ];
        assert_eq!(pick_backend_asset("macos", &assets).unwrap().name, "nscb_rust-macos-arm64");
        assert_eq!(pick_backend_asset("linux", &assets).unwrap().name, "nscb_rust-linux-amd64");
        assert_eq!(pick_backend_asset("windows", &assets).unwrap().name, "nscb_rust.exe");
    }

    #[test]
    fn tag_comparison_ignores_v_prefix() {
        assert!(!is_newer_tag("v2.7.2", "2.7.2"));
        assert!(is_newer_tag("v2.8.0", "2.7.2"));
    }
}
