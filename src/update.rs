//! Looks for a newer release of the app on GitHub. Nothing is downloaded or installed: the
//! tray menu only offers a link to the download.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

const LATEST_RELEASE: &str = "https://api.github.com/repos/allgoewer/coh3-display/releases/latest";
/// Where the download is. A fixed address, so the app never opens one a server handed it.
pub const DOWNLOAD_URL: &str = "https://github.com/allgoewer/coh3-display/releases/latest";

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
}

/// `v1.3.0` -> `1.3.0`. Anything but a plain release tag is no version to offer.
fn tag_version(tag: &str) -> Option<&str> {
    let version = tag.strip_prefix('v')?;
    let parts: Vec<&str> = version.split('.').collect();
    let numeric = parts.iter().all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()));
    (parts.len() == 3 && numeric).then_some(version)
}

fn version_key(version: &str) -> Vec<u64> {
    version.split('.').map(|part| part.parse().unwrap_or(0)).collect()
}

fn is_newer(latest: &str, current: &str) -> bool {
    version_key(latest) > version_key(current)
}

/// What a response from GitHub means for an app of version `current`.
fn newer_than(current: &str, response: &str) -> Result<Option<String>> {
    let release: Release = serde_json::from_str(response).context("unexpected response from GitHub")?;
    let latest = tag_version(&release.tag_name).with_context(|| format!("odd release tag {}", release.tag_name))?;
    Ok(is_newer(latest, current).then(|| latest.to_string()))
}

/// The latest release, if it is newer than this app: `Some("1.3.0")`. One request, no retry.
pub async fn newer_release() -> Result<Option<String>> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("coh3-display/", env!("CARGO_PKG_VERSION")))
        .build()?;
    // The newest release that is neither a draft nor a pre-release.
    let response = http.get(LATEST_RELEASE).send().await?.error_for_status()?.text().await?;
    newer_than(env!("CARGO_PKG_VERSION"), &response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_release_tags_count() {
        assert_eq!(tag_version("v1.3.0"), Some("1.3.0"));
        for tag in ["1.3.0", "v1.3", "v1.3.0-rc.1", "v1.3.0.1", "v1..0", "nightly", ""] {
            assert_eq!(tag_version(tag), None, "{tag}");
        }
    }

    #[test]
    fn compares_versions_number_by_number() {
        assert!(is_newer("1.3.0", "1.2.1"));
        assert!(is_newer("2.0.0", "1.9.9"));
        // Not as text, where 1.10.0 would sort before 1.9.0.
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(!is_newer("1.2.1", "1.2.1"));
        assert!(!is_newer("1.2.0", "1.2.1"));
    }

    #[test]
    fn reads_the_latest_release_from_githubs_answer() {
        let response = r#"{"url": "https://api.github.com/repos/x/y/releases/1", "tag_name": "v1.3.0",
            "name": "v1.3.0", "draft": false, "prerelease": false, "assets": []}"#;
        assert_eq!(newer_than("1.2.1", response).unwrap(), Some("1.3.0".to_string()));
        assert_eq!(newer_than("1.3.0", response).unwrap(), None);
        // A build that is ahead of the latest release has nothing to be offered.
        assert_eq!(newer_than("1.4.0", response).unwrap(), None);

        assert!(newer_than("1.2.1", r#"{"message": "Not Found"}"#).is_err());
        assert!(newer_than("1.2.1", r#"{"tag_name": "nightly"}"#).is_err());
    }
}
