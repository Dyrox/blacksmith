//! 检查更新: the latest GitHub release against this build's version.
//!
//! CI builds carry their version (`BLACKSMITH_VERSION`, set by the release
//! workflow); local builds have none and don't check. The request goes through
//! the system's `curl` (macOS, Windows 10+), so no HTTP or TLS crate is needed,
//! and it sends nothing but a generic user agent.

use std::process::Command;

use crate::folders::Json;

const LATEST: &str = "https://api.github.com/repos/Dyrox/blacksmith/releases/latest";

/// A newer release than this build.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    /// Its page on GitHub, with the downloads.
    pub url: String,
}

/// This build's version. Debug aid: `BLACKSMITH_VERSION` at run time
/// overrides it (e.g. `0.1.0` to see the update notice).
pub fn current() -> Option<String> {
    std::env::var("BLACKSMITH_VERSION").ok().or(option_env!("BLACKSMITH_VERSION").map(str::to_string))
}

/// The latest release when it's newer than this build; `None` when it isn't,
/// for a local build, or when GitHub can't be reached.
pub fn check() -> Option<Release> {
    let current = current()?;
    let latest = latest()?;
    (number(&latest.version)? > number(&current)?).then_some(latest)
}

fn latest() -> Option<Release> {
    let mut curl = Command::new("curl");
    curl.args(["-fsSL", "--max-time", "10", "-A", "Blacksmith", "-H", "Accept: application/vnd.github+json", LATEST]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        curl.creation_flags(CREATE_NO_WINDOW);
    }
    let out = curl.output().ok().filter(|o| o.status.success())?;
    parse(std::str::from_utf8(&out.stdout).ok()?)
}

/// GitHub's release JSON → version (tag without its `v`) and page.
fn parse(json: &str) -> Option<Release> {
    let json = Json::parse(json)?;
    let tag = json.get("tag_name")?.str()?;
    let url = json.get("html_url")?.str()?;
    Some(Release { version: tag.trim_start_matches('v').to_string(), url: url.to_string() })
}

/// `0.1.10` → `[0, 1, 10]`, compared part by part.
fn number(version: &str) -> Option<Vec<u32>> {
    version.trim_start_matches('v').split('.').map(|p| p.parse().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_compares() {
        let r = parse(r#"{"tag_name": "v0.1.10", "html_url": "https://github.com/Dyrox/blacksmith/releases/tag/v0.1.10", "assets": [], "draft": false}"#).unwrap();
        assert_eq!(r.version, "0.1.10");
        assert!(r.url.ends_with("/v0.1.10"));
        // 0.1.10 is newer than 0.1.9 (by number, not by text).
        assert!(number("0.1.10") > number("0.1.9"));
        assert!(number("v0.2.0") > number("0.1.99"));
        assert_eq!(number("0.1.x"), None);
    }
}
