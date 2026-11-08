//! GitHub release lookup for the app-update notice. Blocking; run it on a
//! background executor.

use serde::Deserialize;

const APP_RELEASES: &str = "https://api.github.com/repos/cxfcxf/nscb-desktop/releases/latest";
const USER_AGENT: &str = concat!("nscb-desktop/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub tag: String,
    /// Release page URL.
    pub url: String,
}

#[derive(Deserialize)]
struct ApiRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    html_url: String,
}

pub fn latest_app_release() -> Option<Release> {
    let release: ApiRelease = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .ok()?
        .get(APP_RELEASES)
        .header("Accept", "application/vnd.github+json")
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .ok()?;
    if release.tag_name.is_empty() {
        return None;
    }
    Some(Release { tag: release.tag_name, url: release.html_url })
}

/// Whether a release tag (`v1.2.3` or `1.2.3`) differs from the running version.
pub fn is_newer_tag(tag: &str, current: &str) -> bool {
    tag.trim_start_matches('v') != current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_comparison_ignores_v_prefix() {
        assert!(!is_newer_tag("v2.7.2", "2.7.2"));
        assert!(is_newer_tag("v2.8.0", "2.7.2"));
    }
}
