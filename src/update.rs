//! Checking GitHub for a newer release of the app, in the background.
//!
//! The check asks GitHub's public releases API for this app's latest release. Nothing
//! about the user or their maps is sent: the request only names the app and its version.
//! While the app is up to date it checks at most once a day; it can be turned off.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use thiserror::Error;

/// GitHub's description of the latest (non-pre-release) release.
const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/BrandonG2001/mind_mapper/releases/latest";
/// Where release pages live; a reply pointing anywhere else is not trusted.
const RELEASES_PAGE: &str = "https://github.com/BrandonG2001/mind_mapper/releases";
/// Give up on a slow or unreachable network after this long.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Minimum time between checks while the app is up to date.
pub const CHECK_INTERVAL_SECONDS: u64 = 24 * 60 * 60;

/// Errors that can occur while checking for an update.
#[derive(Debug, Error)]
pub enum UpdateError {
    /// GitHub could not be reached, or refused the request.
    #[error("could not reach GitHub: {0}")]
    Network(#[from] ureq::Error),
    /// The reply was not the expected JSON.
    #[error("unexpected reply from GitHub: {0}")]
    Reply(#[from] serde_json::Error),
    /// The latest release's tag is not a version number.
    #[error("the latest release is not named like a version: {0}")]
    BadVersion(String),
}

/// A release version, `major.minor.patch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    /// The version of this build of the app.
    ///
    /// # Panics
    ///
    /// Only if `Cargo.toml`'s version is not `major.minor.patch`, which a test rules out.
    #[must_use]
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).expect("Cargo.toml's version is major.minor.patch")
    }

    /// Read a version such as `0.7.0` or a tag such as `v0.7.0`.
    ///
    /// # Returns
    ///
    /// `None` for anything else, including pre-releases such as `1.0.0-beta`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = text.split('.').map(|part| part.parse::<u64>().ok());
        let version = Self {
            major: parts.next()??,
            minor: parts.next()??,
            patch: parts.next()??,
        };
        // Exactly three numbers.
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A newer release, and the page to download it from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Its version.
    pub version: Version,
    /// Its page on GitHub.
    pub page: String,
}

/// Where the latest release is looked up: GitHub, or a stand-in in tests.
pub trait ReleaseSource {
    /// The JSON GitHub's "latest release" API returns.
    ///
    /// # Errors
    ///
    /// Returns `UpdateError::Network` if it cannot be fetched.
    fn latest_release_json(&self) -> Result<String, UpdateError>;
}

/// GitHub's public releases API.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitHub;

impl ReleaseSource for GitHub {
    fn latest_release_json(&self) -> Result<String, UpdateError> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            // GitHub requires a user agent; this names only the app and its version.
            .user_agent(format!("mind-mapper/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Ok(agent
            .get(LATEST_RELEASE_URL)
            .header("Accept", "application/vnd.github+json")
            .call()?
            .body_mut()
            .read_to_string()?)
    }
}

/// Ask `source` for the latest release.
///
/// # Returns
///
/// The release if it is newer than `current`, else `None`.
///
/// # Errors
///
/// * `UpdateError::Network` if `source` cannot be reached
/// * `UpdateError::Reply` or `UpdateError::BadVersion` if its answer makes no sense
pub fn newer_release(
    source: &impl ReleaseSource,
    current: Version,
) -> Result<Option<Release>, UpdateError> {
    /// The two fields used from GitHub's reply (the rest is ignored).
    #[derive(Deserialize)]
    struct Latest {
        tag_name: String,
        html_url: String,
    }
    let latest: Latest = serde_json::from_str(&source.latest_release_json()?)?;
    let version =
        Version::parse(&latest.tag_name).ok_or(UpdateError::BadVersion(latest.tag_name))?;
    // Only ever open this project's own release pages.
    let page = if latest.html_url.starts_with(RELEASES_PAGE) {
        latest.html_url
    } else {
        RELEASES_PAGE.to_owned()
    };
    Ok((version > current).then_some(Release { version, page }))
}

/// Whether a check is due, given when the last one found the app up to date (seconds
/// since 1970) and the time now: never, a day or more ago, or in the future (the clock
/// was changed).
#[must_use]
pub fn is_due(last_check: Option<u64>, now: u64) -> bool {
    last_check.is_none_or(|last| now < last || now - last >= CHECK_INTERVAL_SECONDS)
}

/// The time now, in seconds since 1970 (0 if the system clock is before that).
#[must_use]
pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers with fixed JSON instead of asking GitHub.
    struct Canned(&'static str);

    impl ReleaseSource for Canned {
        fn latest_release_json(&self) -> Result<String, UpdateError> {
            Ok(self.0.to_owned())
        }
    }

    fn version(text: &str) -> Version {
        Version::parse(text).expect("valid version")
    }

    #[test]
    fn versions_parse_and_compare_numerically() {
        assert_eq!(version("v0.10.0").to_string(), "0.10.0");
        assert!(version("0.10.0") > version("0.9.9"));
        assert!(version("1.0.0") > version("0.99.99"));
        for invalid in ["", "1.0", "1.0.0.0", "1.0.0-beta", "v1.x.0", "latest"] {
            assert_eq!(Version::parse(invalid), None, "{invalid}");
        }
        // The app's own version must parse, or `current` would panic.
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn only_newer_releases_are_reported() {
        let reply = Canned(
            r#"{"tag_name": "v0.8.0", "name": "x", "html_url": "https://github.com/BrandonG2001/mind_mapper/releases/tag/v0.8.0"}"#,
        );

        let found = newer_release(&reply, version("0.7.0")).expect("valid reply");

        assert_eq!(
            found,
            Some(Release {
                version: version("0.8.0"),
                page: "https://github.com/BrandonG2001/mind_mapper/releases/tag/v0.8.0".to_owned(),
            })
        );
        assert_eq!(
            newer_release(&reply, version("0.8.0")).expect("valid"),
            None
        );
        assert_eq!(
            newer_release(&reply, version("0.9.0")).expect("valid"),
            None
        );
    }

    #[test]
    fn odd_replies_are_errors_or_made_safe() {
        let elsewhere = Canned(r#"{"tag_name": "v9.0.0", "html_url": "https://example.com/x"}"#);
        let found = newer_release(&elsewhere, version("0.1.0")).expect("valid");
        assert_eq!(
            found.map(|release| release.page).as_deref(),
            Some(RELEASES_PAGE)
        );

        assert!(matches!(
            newer_release(&Canned("not json"), version("0.1.0")),
            Err(UpdateError::Reply(_))
        ));
        assert!(matches!(
            newer_release(
                &Canned(r#"{"tag_name": "nightly", "html_url": ""}"#),
                version("0.1.0")
            ),
            Err(UpdateError::BadVersion(_))
        ));
    }

    /// Talks to the real GitHub API, so it only runs when asked for:
    /// `cargo test -- --ignored live_github`.
    #[test]
    #[ignore = "needs the network"]
    fn live_github_reports_the_published_release() {
        let found = newer_release(&GitHub, version("0.0.1")).expect("GitHub answers");

        let release = found.expect("some release is newer than 0.0.1");
        assert!(release.page.starts_with(RELEASES_PAGE), "{}", release.page);
    }

    #[test]
    fn checks_are_due_once_a_day() {
        let day = CHECK_INTERVAL_SECONDS;
        assert!(is_due(None, 1_000));
        assert!(!is_due(Some(1_000), 1_000 + day - 1));
        assert!(is_due(Some(1_000), 1_000 + day));
        // The clock was set back.
        assert!(is_due(Some(1_000), 999));
    }
}
