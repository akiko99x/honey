//! Self-update: check GitHub releases and, on explicit operator action, download
//! + verify + stage a new master binary.
//!
//! This is a remote-code path into the control plane, so it is deliberately
//! conservative:
//!   * **disabled by default** (`self_update_enabled` runtime setting),
//!   * owner-only and audited at the API layer,
//!   * **SHA-256 verification is mandatory** — a release without a checksums
//!     asset is refused rather than trusted,
//!   * never silent: nothing is downloaded or swapped without a request.
//!
//! The binary is staged next to the running one and renamed into place; the
//! actual restart belongs to the supervisor (systemd `Restart=always`), which is
//! what makes a rolling, zero-downtime upgrade possible together with HA.
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// `owner/repo` to check; overridable so forks and private mirrors work.
pub fn repo() -> String {
    std::env::var("HONEY_UPDATE_REPO")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "akiko99x/honey".to_string())
}

#[derive(Debug, Serialize)]
pub struct UpdateStatus {
    pub current: String,
    pub latest: Option<String>,
    pub newer: bool,
    pub notes: String,
    pub asset: Option<String>,
    pub published_at: Option<String>,
    pub repo: String,
}

#[derive(Debug, Clone)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct Release {
    pub tag: String,
    pub notes: String,
    pub published_at: Option<String>,
    pub assets: Vec<ReleaseAsset>,
}

/// Numeric version triple, ignoring a leading `v`. A pre-release suffix marks
/// the version as *older* than the same triple without one.
fn parse_version(value: &str) -> (u64, u64, u64, bool) {
    let raw = value.trim().trim_start_matches('v');
    let (core, pre) = match raw.split_once('-') {
        Some((core, _)) => (core, true),
        None => (raw, false),
    };
    let mut parts = core
        .split('.')
        .map(|p| p.trim().parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        pre,
    )
}

/// True when `latest` is strictly newer than `current`.
pub fn is_newer(current: &str, latest: &str) -> bool {
    let (a1, a2, a3, a_pre) = parse_version(current);
    let (b1, b2, b3, b_pre) = parse_version(latest);
    match (b1, b2, b3).cmp(&(a1, a2, a3)) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        // same triple: a release build beats the pre-release we are running.
        std::cmp::Ordering::Equal => a_pre && !b_pre,
    }
}

/// Pick the asset built for this platform.
pub fn pick_asset<'a>(assets: &'a [ReleaseAsset]) -> Option<&'a ReleaseAsset> {
    let os = std::env::consts::OS; // "linux", "windows", ...
    let arch_aliases: &[&str] = match std::env::consts::ARCH {
        "x86_64" => &["x86_64", "amd64", "x64"],
        "aarch64" => &["aarch64", "arm64"],
        other => return assets.iter().find(|a| a.name.contains(other)),
    };
    assets.iter().find(|a| {
        let name = a.name.to_ascii_lowercase();
        !is_checksum_asset(&name)
            && name.contains(os)
            && arch_aliases.iter().any(|arch| name.contains(arch))
    })
}

fn is_checksum_asset(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("sha256") || name.contains("checksum")
}

/// Find the expected hash for `asset_name` inside a `sha256sum`-style listing
/// (`<hex>  <filename>` per line).
pub fn expected_hash(checksums: &str, asset_name: &str) -> Option<String> {
    for line in checksums.lines() {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let file = parts.next().unwrap_or("");
        let file = file.trim_start_matches('*');
        if file.ends_with(asset_name) && hash.len() == 64 {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .user_agent(format!("honey-master/{CURRENT_VERSION}"))
        .build()?)
}

/// Fetch the newest published release from GitHub.
pub async fn latest_release() -> Result<Release> {
    let repo = repo();
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let body: serde_json::Value = client()?
        .get(&url)
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .with_context(|| format!("contacting {url}"))?
        .error_for_status()
        .context("github returned an error status")?
        .json()
        .await
        .context("parsing the github release payload")?;

    let assets = body["assets"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|a| {
                    Some(ReleaseAsset {
                        name: a["name"].as_str()?.to_string(),
                        url: a["browser_download_url"].as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Release {
        tag: body["tag_name"].as_str().unwrap_or_default().to_string(),
        notes: body["body"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(4000)
            .collect(),
        published_at: body["published_at"].as_str().map(|s| s.to_string()),
        assets,
    })
}

/// Check-only: never downloads anything.
pub async fn check() -> Result<UpdateStatus> {
    let release = latest_release().await?;
    let newer = is_newer(CURRENT_VERSION, &release.tag);
    Ok(UpdateStatus {
        current: CURRENT_VERSION.to_string(),
        newer,
        asset: pick_asset(&release.assets).map(|a| a.name.clone()),
        latest: Some(release.tag),
        notes: release.notes,
        published_at: release.published_at,
        repo: repo(),
    })
}

/// Download the platform asset, verify its SHA-256 against the release's
/// checksums asset, and swap it over the running binary. Returns the version
/// that is now staged. The caller restarts (or lets the supervisor restart) to
/// actually run it.
pub async fn apply() -> Result<String> {
    let release = latest_release().await?;
    if !is_newer(CURRENT_VERSION, &release.tag) {
        bail!(
            "already running {CURRENT_VERSION}; latest release is {}",
            release.tag
        );
    }
    let asset = pick_asset(&release.assets)
        .ok_or_else(|| anyhow!("release {} has no asset for this platform", release.tag))?
        .clone();
    let checksums = release
        .assets
        .iter()
        .find(|a| is_checksum_asset(&a.name))
        .ok_or_else(|| {
            anyhow!(
                "release {} publishes no checksums asset — refusing to trust the binary",
                release.tag
            )
        })?
        .clone();

    let http = client()?;
    let checksum_body = http
        .get(&checksums.url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let expected = expected_hash(&checksum_body, &asset.name)
        .ok_or_else(|| anyhow!("no SHA-256 entry for {} in {}", asset.name, checksums.name))?;

    let bytes = http
        .get(&asset.url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected {
        bail!(
            "checksum mismatch for {}: expected {expected}, got {actual}",
            asset.name
        );
    }

    let current = std::env::current_exe().context("locating the running binary")?;
    let staged = current.with_extension("new");
    std::fs::write(&staged, &bytes).with_context(|| format!("writing {}", staged.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
            .context("marking the staged binary executable")?;
    }
    // rename is atomic within a filesystem; on Linux replacing a running binary
    // is fine (the old inode stays alive until this process exits).
    std::fs::rename(&staged, &current)
        .with_context(|| format!("installing {}", current.display()))?;
    tracing::warn!(
        code = "M0122",
        "self-update staged {} over {} — restart to run it",
        release.tag,
        current.display()
    );
    Ok(release.tag)
}

/// Ask the supervisor to restart after the HTTP response has been sent.
/// Production systemd units opt in with HONEY_UPDATE_AUTO_RESTART=1. Keeping
/// the switch explicit makes local/manual launches safe.
pub fn schedule_restart() {
    if std::env::var("HONEY_UPDATE_AUTO_RESTART").ok().as_deref() != Some("1") {
        return;
    }
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        tracing::warn!(code = "M0122", "self-update restart requested");
        std::process::exit(0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("0.1.0", "v0.1.1"));
        assert!(is_newer("1.0.0", "1.0.1"));
        assert!(!is_newer("0.2.0", "0.1.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        // a final release beats the pre-release of the same triple
        assert!(is_newer("1.2.3-rc1", "1.2.3"));
        assert!(!is_newer("1.2.3", "1.2.3-rc1"));
    }

    #[test]
    fn picks_platform_asset_and_skips_checksums() {
        let assets = vec![
            ReleaseAsset {
                name: "SHA256SUMS".into(),
                url: "u0".into(),
            },
            ReleaseAsset {
                name: format!(
                    "honey-master-{}-{}",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ),
                url: "u1".into(),
            },
            ReleaseAsset {
                name: "honey-master-otheros-otherarch".into(),
                url: "u2".into(),
            },
        ];
        let picked = pick_asset(&assets).expect("an asset for the host platform");
        assert_eq!(picked.url, "u1");
        assert!(!is_checksum_asset(&picked.name));
    }

    #[test]
    fn parses_checksum_listing() {
        let listing = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  honey-master-linux-x86_64
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb *dist/honey-master-linux-arm64
";
        assert_eq!(
            expected_hash(listing, "honey-master-linux-x86_64").unwrap(),
            "a".repeat(64)
        );
        assert_eq!(
            expected_hash(listing, "honey-master-linux-arm64").unwrap(),
            "b".repeat(64)
        );
        assert!(expected_hash(listing, "missing-asset").is_none());
    }
}
