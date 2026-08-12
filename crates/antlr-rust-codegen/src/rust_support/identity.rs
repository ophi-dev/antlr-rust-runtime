// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;

use hmac_sha256::Hash;
use serde::{Deserialize, Serialize};

use crate::json::to_pretty_json;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BundleIdentity {
    pub(crate) repository: String,
    pub(crate) revision: Option<String>,
    pub(crate) bundle_path: String,
    pub(crate) fingerprint: String,
}

impl BundleIdentity {
    pub(crate) fn revision_key(&self) -> String {
        format!(
            "{}@{}#{}",
            self.repository,
            self.revision.as_deref().unwrap_or("unversioned"),
            self.fingerprint
        )
    }

    pub(crate) fn source_label(&self) -> String {
        if self.bundle_path.is_empty() {
            self.repository.clone()
        } else {
            format!("{}/{}", self.repository, self.bundle_path)
        }
    }

    pub(crate) fn artifact_id(&self) -> String {
        let fingerprint = self
            .fingerprint
            .strip_prefix("sha256:")
            .expect("fingerprints are normalized");
        let mut digest = Hash::new();
        digest.update(self.repository.as_bytes());
        digest.update(b"\0");
        digest.update(self.revision.as_deref().unwrap_or("unversioned").as_bytes());
        digest.update(b"\0");
        digest.update(self.bundle_path.as_bytes());
        let source = hex_lower(&digest.finalize());
        format!("{}-{}", &fingerprint[..12], &source[..8])
    }
}

pub(crate) fn identify(
    grammar_directory: &Path,
    support_directory: &Path,
    fingerprint_root: &Path,
) -> io::Result<BundleIdentity> {
    let fingerprint = fingerprint_directory(fingerprint_root)?;
    let git_root = git_output(grammar_directory, ["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    let repository = git_output(grammar_directory, ["config", "--get", "remote.origin.url"])
        .map(|remote| normalize_repository(&remote))
        .or_else(|| git_root.as_ref().map(|path| path.display().to_string()))
        .unwrap_or_else(|| grammar_directory.display().to_string());
    let revision = git_output(grammar_directory, ["rev-parse", "HEAD"]);
    let bundle_path = git_root
        .as_deref()
        .and_then(|root| support_directory.strip_prefix(root).ok())
        .map(path_key)
        .transpose()?
        .unwrap_or_else(|| {
            support_directory
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or("Rust")
                .to_owned()
        });
    Ok(BundleIdentity {
        repository,
        revision,
        bundle_path,
        fingerprint,
    })
}

pub(crate) fn fingerprint_directory(directory: &Path) -> io::Result<String> {
    let mut files = Vec::new();
    collect_files(directory, directory, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut digest = Hash::new();
    digest.update(b"antlr-rust-support-v1\0");
    for (relative, path) in files {
        let contents = fs::read(path)?;
        let relative_len = u64::try_from(relative.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Rust support relative path length exceeds u64",
            )
        })?;
        let contents_len = u64::try_from(contents.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Rust support file length exceeds u64",
            )
        })?;
        digest.update(relative_len.to_be_bytes());
        digest.update(relative.as_bytes());
        digest.update(contents_len.to_be_bytes());
        digest.update(contents);
    }
    Ok(format!("sha256:{}", hex_lower(&digest.finalize())))
}

fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Rust target-support bundles may not contain symlinks: {}",
                    path.display()
                ),
            ));
        }
        if file_type.is_dir() {
            collect_files(root, &path, files)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("collected path stays below support root");
            files.push((path_key(relative)?, path));
        }
    }
    Ok(())
}

fn path_key(path: &Path) -> io::Result<String> {
    let text = path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Rust target-support path is not UTF-8: {}", path.display()),
        )
    })?;
    Ok(text
        .chars()
        .map(|character| {
            if character == std::path::MAIN_SEPARATOR {
                '/'
            } else {
                character
            }
        })
        .collect())
}

fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn git_output<const N: usize>(directory: &Path, args: [&str; N]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = std::str::from_utf8(&output.stdout).ok()?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn normalize_repository(remote: &str) -> String {
    let remote = remote.trim().trim_end_matches(".git");
    if let Some((host, path)) = remote
        .strip_prefix("git@")
        .and_then(|value| value.split_once(':'))
    {
        return format!("https://{host}/{path}");
    }
    if let Some((scheme, remainder)) = remote.split_once("://") {
        let authority_end = remainder.find('/').unwrap_or(remainder.len());
        let (authority, path) = remainder.split_at(authority_end);
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        let scheme = if scheme == "ssh" { "https" } else { scheme };
        return format!("{scheme}://{host}{path}");
    }
    remote.to_owned()
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct TrustFile {
    version: u32,
    repositories: BTreeSet<String>,
    revisions: BTreeSet<String>,
}

#[derive(Debug)]
pub(crate) struct TrustStore {
    path: Option<PathBuf>,
    file: TrustFile,
}

impl TrustStore {
    pub(crate) fn load(path: Option<PathBuf>) -> io::Result<Self> {
        let path = path.or_else(default_trust_store_path);
        let file = read_trust_file(path.as_deref())?;
        Ok(Self { path, file })
    }

    pub(crate) fn trusts_revision(&self, identity: &BundleIdentity) -> bool {
        self.file.revisions.contains(&identity.revision_key())
    }

    pub(crate) fn trusts_repository(&self, identity: &BundleIdentity) -> bool {
        self.file.repositories.contains(&identity.repository)
    }

    pub(crate) fn trust_revision(&mut self, identity: &BundleIdentity) -> io::Result<()> {
        self.file.revisions.insert(identity.revision_key());
        self.save()
    }

    pub(crate) fn trust_repository(&mut self, identity: &BundleIdentity) -> io::Result<()> {
        self.file.repositories.insert(identity.repository.clone());
        self.save()
    }

    fn save(&self) -> io::Result<()> {
        let path = self.path.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "cannot persist Rust support trust without a home/config directory",
            )
        })?;
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("trust store has no parent directory: {}", path.display()),
            )
        })?;
        fs::create_dir_all(parent)?;
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .truncate(false)
            .write(true)
            .open(PathBuf::from(lock_path))?;
        lock.lock()?;

        let mut merged = read_trust_file(Some(path))?;
        merged
            .repositories
            .extend(self.file.repositories.iter().cloned());
        merged.revisions.extend(self.file.revisions.iter().cloned());
        let contents = to_pretty_json(&merged);
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(contents.as_bytes())?;
        temporary
            .persist(path)
            .map_err(|error| error.error)
            .map(drop)
    }
}

fn read_trust_file(path: Option<&Path>) -> io::Result<TrustFile> {
    let contents = match path.map(fs::read_to_string).transpose() {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let file = match contents {
        Some(contents) => serde_json::from_str::<TrustFile>(&contents).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid Rust support trust store: {error}"),
            )
        })?,
        None => TrustFile {
            version: 1,
            ..TrustFile::default()
        },
    };
    if file.version != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported Rust support trust store version {}",
                file.version
            ),
        ));
    }
    Ok(file)
}

fn default_trust_store_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(
            PathBuf::from(path)
                .join("antlr4-rust")
                .join("trusted-support.json"),
        );
    }
    #[cfg(target_os = "windows")]
    if let Some(path) = std::env::var_os("APPDATA") {
        return Some(
            PathBuf::from(path)
                .join("antlr4-rust")
                .join("trusted-support.json"),
        );
    }
    #[cfg(target_os = "windows")]
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)?;
    #[cfg(not(target_os = "windows"))]
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    #[cfg(target_os = "macos")]
    return Some(
        home.join("Library")
            .join("Application Support")
            .join("antlr4-rust")
            .join("trusted-support.json"),
    );
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Some(
        home.join(".config")
            .join("antlr4-rust")
            .join("trusted-support.json"),
    );
    #[cfg(target_os = "windows")]
    Some(
        home.join("AppData")
            .join("Roaming")
            .join("antlr4-rust")
            .join("trusted-support.json"),
    )
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
mod tests {
    use std::fs;
    use std::sync::{Arc, Barrier};

    use super::{BundleIdentity, TrustStore, fingerprint_directory, normalize_repository};

    #[test]
    fn repository_identity_strips_remote_credentials() {
        assert_eq!(
            normalize_repository("https://build:secret@github.com/example/grammars.git"),
            "https://github.com/example/grammars"
        );
        assert_eq!(
            normalize_repository("ssh://git@github.com/example/grammars.git"),
            "https://github.com/example/grammars"
        );
        assert_eq!(
            normalize_repository("git@github.com:example/grammars.git"),
            "https://github.com/example/grammars"
        );
    }

    #[test]
    fn persisted_scopes_round_trip() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let path = temporary.path().join("trusted-support.json");
        let identity = BundleIdentity {
            repository: "https://example.test/grammars".to_owned(),
            revision: Some("0123456789abcdef".to_owned()),
            bundle_path: "grammar/Rust".to_owned(),
            fingerprint: format!("sha256:{}", "a".repeat(64)),
        };
        let mut store = TrustStore::load(Some(path.clone())).expect("empty store should load");
        assert!(!store.trusts_revision(&identity));
        assert!(!store.trusts_repository(&identity));

        store
            .trust_revision(&identity)
            .expect("revision trust should persist");
        let store = TrustStore::load(Some(path.clone())).expect("revision store should reload");
        assert!(store.trusts_revision(&identity));
        assert!(!store.trusts_repository(&identity));

        let mut store = store;
        store
            .trust_repository(&identity)
            .expect("repository trust should persist");
        let store = TrustStore::load(Some(path.clone())).expect("repository store should reload");
        assert!(store.trusts_repository(&identity));
        insta::assert_snapshot!(
            "persisted_trust_store",
            fs::read_to_string(path).expect("trust store should remain readable")
        );
    }

    #[test]
    fn concurrent_writers_merge_trust_decisions() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let path = temporary.path().join("trusted-support.json");
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for repository in ["https://example.test/first", "https://example.test/second"] {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let identity = BundleIdentity {
                    repository: repository.to_owned(),
                    revision: Some("0123456789abcdef".to_owned()),
                    bundle_path: "grammar/Rust".to_owned(),
                    fingerprint: format!("sha256:{}", "a".repeat(64)),
                };
                let mut store = TrustStore::load(Some(path)).expect("concurrent store should load");
                barrier.wait();
                store
                    .trust_repository(&identity)
                    .expect("concurrent trust should persist");
            }));
        }
        for handle in handles {
            handle.join().expect("trust writer should not panic");
        }

        let store = TrustStore::load(Some(path)).expect("merged store should load");
        for repository in ["https://example.test/first", "https://example.test/second"] {
            let identity = BundleIdentity {
                repository: repository.to_owned(),
                revision: Some("0123456789abcdef".to_owned()),
                bundle_path: "grammar/Rust".to_owned(),
                fingerprint: format!("sha256:{}", "a".repeat(64)),
            };
            assert!(store.trusts_repository(&identity));
        }
    }

    #[test]
    fn artifact_ids_disambiguate_identical_bundle_contents() {
        let fingerprint = format!("sha256:{}", "a".repeat(64));
        let first = BundleIdentity {
            repository: "https://example.test/grammars".to_owned(),
            revision: Some("0123456789abcdef".to_owned()),
            bundle_path: "first/Rust".to_owned(),
            fingerprint: fingerprint.clone(),
        };
        let second = BundleIdentity {
            repository: first.repository.clone(),
            revision: first.revision.clone(),
            bundle_path: "second/Rust".to_owned(),
            fingerprint,
        };

        assert_ne!(first.artifact_id(), second.artifact_id());
    }

    #[test]
    fn identity_hashes_match_legacy_sha2_values() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let nested = temporary.path().join("nested");
        fs::create_dir(&nested).expect("nested support directory");
        fs::write(temporary.path().join("Grammar.g4"), "grammar Example;\n")
            .expect("root support file");
        fs::write(nested.join("support.rs"), "pub struct Support;\n").expect("nested support file");

        let identity = BundleIdentity {
            repository: "https://example.test/grammars".to_owned(),
            revision: Some("0123456789abcdef".to_owned()),
            bundle_path: "example/Rust".to_owned(),
            fingerprint: fingerprint_directory(temporary.path())
                .expect("support directory fingerprint"),
        };

        insta::assert_debug_snapshot!(
            "legacy_sha2_identity_values",
            (&identity.fingerprint, identity.artifact_id())
        );
    }
}
