//! Lightweight GitHub-releases update check. Hits the public Releases
//! API once on startup, finds the newest non-prerelease tag matching
//! `vX.Y.Z`, and reports it back if it's newer than the running
//! `CARGO_PKG_VERSION`. Also pulls out the platform-specific download
//! asset URL so the in-app updater can fetch it without extra round
//! trips — letting users jump straight to the freshest GitHub release
//! instead of waiting on Microsoft Store / App Store certification.

use serde::Deserialize;

const REPO: &str = "zdavatz/eudamed2firstbase";
const TAG_PREFIX: &str = "v";
const USER_AGENT: &str = "eudamed2firstbase-update-check";

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Clone, Debug)]
pub struct UpdateInfo {
    pub version: (u32, u32, u32),
    pub url: String,
    /// Direct download URL for the platform-specific release artifact:
    /// `-macos-universal.dmg` on macOS, the matching `-linux-x86_64.tar.gz`
    /// on x86_64 Linux, the matching `-windows-x64.zip` on x86_64 Windows.
    /// None when the release page hasn't published the artifact for this
    /// target yet.
    pub download_url: Option<String>,
}

impl UpdateInfo {
    pub fn pretty(&self) -> String {
        format!("v{}.{}.{}", self.version.0, self.version.1, self.version.2)
    }
}

pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut parts = s.splitn(3, '.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    let patch_raw = parts.next()?;
    let patch_str: String = patch_raw
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let patch: u32 = patch_str.parse().ok()?;
    Some((major, minor, patch))
}

/// Suffix the platform-specific release asset is expected to end with.
/// Empty string = no in-app update on this target (e.g. aarch64 Linux,
/// which the CI does not publish a GUI artifact for). Mirrors the asset
/// names produced by `.github/workflows/release.yml`.
pub const fn target_asset_suffix() -> &'static str {
    if cfg!(target_os = "macos") {
        // CI ships a single universal (arm64+x86_64) DMG.
        "-macos-universal.dmg"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "-linux-x86_64.tar.gz"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "-windows-x64.zip"
    } else {
        ""
    }
}

fn find_asset(assets: &[GithubAsset]) -> Option<String> {
    let suffix = target_asset_suffix();
    if suffix.is_empty() {
        return None;
    }
    assets
        .iter()
        .find(|a| a.name.ends_with(suffix))
        .map(|a| a.browser_download_url.clone())
}

pub fn check_latest(current: &str) -> Option<UpdateInfo> {
    let cur = parse_version(current)?;
    let url = format!("https://api.github.com/repos/{}/releases?per_page=30", REPO);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .build()
        .into();
    let mut resp = agent
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .ok()?;
    let body = resp.body_mut().read_to_string().ok()?;
    let releases: Vec<GithubRelease> = serde_json::from_str(&body).ok()?;

    let mut best: Option<UpdateInfo> = None;
    for r in releases {
        if r.prerelease {
            continue;
        }
        let Some(stripped) = r.tag_name.strip_prefix(TAG_PREFIX) else {
            continue;
        };
        let Some(v) = parse_version(stripped) else {
            continue;
        };
        if v <= cur {
            continue;
        }
        if best.as_ref().map_or(true, |b| v > b.version) {
            best = Some(UpdateInfo {
                version: v,
                url: r.html_url,
                download_url: find_asset(&r.assets),
            });
        }
    }
    best
}
