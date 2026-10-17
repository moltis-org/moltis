//! Installing the local CA into OS and browser trust stores, and remembering
//! the user's answer per CA so the gateway asks at most once.
//!
//! There is no cross-platform crate for trust-store mutation, so this shells
//! out to the platform tools (`security`, `update-ca-certificates`,
//! `certutil`), the same way mkcert does.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};

use super::{Context, Result};

const TRUST_MARKER_FILE: &str = "ca-trust.txt";
const NSS_NICKNAME: &str = "Moltis Local CA";

/// The user's recorded answer for a specific CA certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustDecision {
    Trusted,
    Declined,
}

impl TrustDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Declined => "declined",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "trusted" => Some(Self::Trusted),
            "declined" => Some(Self::Declined),
            _ => None,
        }
    }
}

/// Outcome of installing the CA into one trust store.
#[derive(Debug)]
pub struct TrustStep {
    pub store: String,
    pub result: std::result::Result<(), String>,
}

/// Per-store outcomes of [`install_ca`].
#[derive(Debug, Default)]
pub struct TrustReport {
    pub steps: Vec<TrustStep>,
    /// Extra guidance when a store could not be handled automatically.
    pub hints: Vec<String>,
}

impl TrustReport {
    /// `true` when at least one store accepted the CA.
    #[must_use]
    pub fn any_installed(&self) -> bool {
        self.steps.iter().any(|step| step.result.is_ok())
    }
}

fn fingerprint(ca_path: &Path) -> Result<String> {
    let pem = std::fs::read(ca_path).context("read CA certificate")?;
    Ok(hex::encode(Sha256::digest(pem)))
}

fn marker_path(ca_path: &Path) -> PathBuf {
    ca_path.with_file_name(TRUST_MARKER_FILE)
}

/// The decision recorded for the CA currently at `ca_path`, if any. A
/// regenerated CA has a new fingerprint and therefore no decision.
#[must_use]
pub fn recorded_decision(ca_path: &Path) -> Option<TrustDecision> {
    let current = fingerprint(ca_path).ok()?;
    let marker = std::fs::read_to_string(marker_path(ca_path)).ok()?;
    let (decision, recorded) = marker.trim().split_once(' ')?;
    (recorded == current)
        .then(|| TrustDecision::parse(decision))
        .flatten()
}

/// Remember `decision` for the CA currently at `ca_path`.
pub fn record_decision(ca_path: &Path, decision: TrustDecision) -> Result<()> {
    let line = format!("{} {}\n", decision.as_str(), fingerprint(ca_path)?);
    std::fs::write(marker_path(ca_path), line).context("write CA trust marker")?;
    Ok(())
}

/// Install the CA into the OS trust store and any NSS browser databases.
///
/// May trigger an OS authentication dialog (macOS, Windows) or a `sudo`
/// password prompt (Linux), so only call this from an interactive session.
pub fn install_ca(ca_path: &Path) -> TrustReport {
    let mut report = TrustReport::default();
    install_os_store(ca_path, &mut report);
    install_nss_stores(ca_path, &mut report);
    if report.any_installed() {
        let _ = record_decision(ca_path, TrustDecision::Trusted);
    }
    report
}

fn run(store: &str, command: &mut Command) -> TrustStep {
    let result = match command.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("exited with {status}")),
        Err(error) => Err(error.to_string()),
    };
    TrustStep {
        store: store.to_string(),
        result,
    }
}

#[cfg(target_os = "macos")]
fn install_os_store(ca_path: &Path, report: &mut TrustReport) {
    let Some(home) = moltis_config::home_dir() else {
        report.hints.push("could not resolve home directory".into());
        return;
    };
    let keychain = home.join("Library/Keychains/login.keychain-db");
    report.steps.push(run(
        "macOS login keychain",
        Command::new("security")
            .args(["add-trusted-cert", "-r", "trustRoot", "-k"])
            .arg(keychain)
            .arg(ca_path),
    ));
}

#[cfg(target_os = "linux")]
fn install_os_store(ca_path: &Path, report: &mut TrustReport) {
    let dest = Path::new("/usr/local/share/ca-certificates/moltis-ca.crt");
    let copy = run(
        "system CA store",
        Command::new("sudo").arg("cp").arg(ca_path).arg(dest),
    );
    if copy.result.is_err() {
        report.steps.push(copy);
        return;
    }
    report.steps.push(run(
        "system CA store",
        Command::new("sudo").arg("update-ca-certificates"),
    ));
}

#[cfg(target_os = "windows")]
fn install_os_store(ca_path: &Path, report: &mut TrustReport) {
    report.steps.push(run(
        "Windows user root store",
        Command::new("certutil")
            .args(["-user", "-addstore", "Root"])
            .arg(ca_path),
    ));
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn install_os_store(ca_path: &Path, report: &mut TrustReport) {
    report.hints.push(format!(
        "automatic trust is not supported on this OS; import {} manually",
        ca_path.display()
    ));
}

/// NSS databases used by Firefox everywhere and by Chromium on Linux.
fn nss_databases(home: &Path) -> Vec<PathBuf> {
    let mut dbs = Vec::new();
    if cfg!(target_os = "linux") {
        dbs.push(home.join(".pki/nssdb"));
    }
    for profiles in [
        home.join(".mozilla/firefox"),
        home.join("snap/firefox/common/.mozilla/firefox"),
        home.join("Library/Application Support/Firefox/Profiles"),
    ] {
        let Ok(entries) = std::fs::read_dir(&profiles) else {
            continue;
        };
        dbs.extend(
            entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| path.join("cert9.db").exists()),
        );
    }
    dbs.into_iter().filter(|db| db.is_dir()).collect()
}

fn install_nss_stores(ca_path: &Path, report: &mut TrustReport) {
    let Some(home) = moltis_config::home_dir() else {
        return;
    };
    let dbs = nss_databases(&home);
    if dbs.is_empty() {
        return;
    }
    if Command::new("certutil").arg("-H").output().is_err() || cfg!(target_os = "windows") {
        report.hints.push(
            "install NSS tools (`certutil`, e.g. libnss3-tools or `brew install nss`) and run \
             `moltis trust-ca` again to trust the CA in Firefox/Chromium"
                .into(),
        );
        return;
    }
    for db in dbs {
        report.steps.push(run(
            &format!("NSS {}", db.display()),
            Command::new("certutil")
                .args(["-A", "-t", "C,,", "-n", NSS_NICKNAME, "-d"])
                .arg(format!("sql:{}", db.display()))
                .arg("-i")
                .arg(ca_path),
        ));
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    fn ca_file(dir: &Path, contents: &str) -> PathBuf {
        let path = dir.join("ca.pem");
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn no_decision_without_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let ca = ca_file(tmp.path(), "ca-one");
        assert_eq!(recorded_decision(&ca), None);
    }

    #[test]
    fn decision_round_trips_for_same_ca() {
        let tmp = tempfile::tempdir().unwrap();
        let ca = ca_file(tmp.path(), "ca-one");
        record_decision(&ca, TrustDecision::Declined).unwrap();
        assert_eq!(recorded_decision(&ca), Some(TrustDecision::Declined));
        record_decision(&ca, TrustDecision::Trusted).unwrap();
        assert_eq!(recorded_decision(&ca), Some(TrustDecision::Trusted));
    }

    #[test]
    fn regenerated_ca_invalidates_decision() {
        let tmp = tempfile::tempdir().unwrap();
        let ca = ca_file(tmp.path(), "ca-one");
        record_decision(&ca, TrustDecision::Trusted).unwrap();
        ca_file(tmp.path(), "ca-two");
        assert_eq!(recorded_decision(&ca), None);
    }

    #[test]
    fn corrupt_marker_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let ca = ca_file(tmp.path(), "ca-one");
        std::fs::write(marker_path(&ca), "maybe abc\n").unwrap();
        assert_eq!(recorded_decision(&ca), None);
    }

    #[test]
    fn nss_databases_finds_firefox_profiles_with_cert_db() {
        let tmp = tempfile::tempdir().unwrap();
        let profile = tmp.path().join(".mozilla/firefox/abc.default");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(profile.join("cert9.db"), "").unwrap();
        std::fs::create_dir_all(tmp.path().join(".mozilla/firefox/empty")).unwrap();

        let dbs = nss_databases(tmp.path());
        assert!(dbs.contains(&profile));
        assert!(!dbs.contains(&tmp.path().join(".mozilla/firefox/empty")));
    }

    #[test]
    fn report_counts_successful_stores() {
        let mut report = TrustReport::default();
        assert!(!report.any_installed());
        report.steps.push(TrustStep {
            store: "a".into(),
            result: Err("nope".into()),
        });
        assert!(!report.any_installed());
        report.steps.push(TrustStep {
            store: "b".into(),
            result: Ok(()),
        });
        assert!(report.any_installed());
    }
}
