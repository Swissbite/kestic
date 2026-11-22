// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Prepared restic repositories for tests.
//!
//! `restic init` costs seconds, and every restic command spends most of its time in the scrypt
//! key derivation. One template repository per restic version and test run is therefore created
//! once and stored with a cheap scrypt key (N=1024, r=1, p=1), as restic's own test suite does.
//! Each test copies it and gets its own password and its own repository ID, so a mix-up of
//! destination secrets or a shared restic cache still makes tests fail.

use crate::password::strong_password;
use crate::random::{hex, random_bytes};
use crate::workspace::find_target_dir;
use aes::cipher::{BlockCipherEncrypt, KeyInit, KeyIvInit, StreamCipher};
use base64::prelude::{BASE64_STANDARD, Engine};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Bumped whenever the template layout changes, so an old template is never reused.
const TEMPLATE_FORMAT: u32 = 1;

/// Cheap scrypt parameters for test keys: log2(N), r, p.
const TEST_KDF: (u8, u32, u32) = (10, 1, 1);

/// A repository copied from the template, with its own password and repository ID.
pub struct TestRepo {
    pub path: PathBuf,
    pub password: String,
    pub repo_id: String,
}

/// Copies the template repository of `restic`'s version to `dest` (created if missing, must be
/// empty), then gives the copy a fresh [`strong_password`] and a fresh repository ID.
pub fn copy_template_repo(restic: &Path, dest: &Path) -> TestRepo {
    let (template, template_password) = template_repo(restic);
    copy_dir(&template, dest);

    let (key_path, key_json) = read_single_key(dest);
    let master = open_key(&key_json, &template_password);
    let password = strong_password();
    write_key(dest, &key_json, &master, &password);
    fs::remove_file(&key_path).expect("remove template key");

    let repo_id = hex(&random_bytes(32));
    rewrite_config_id(dest, &master, &repo_id);
    TestRepo {
        path: dest.to_path_buf(),
        password,
        repo_id,
    }
}

/// Rewrites snapshot `snapshot_id` of the local repository `repo` as if user `username` with
/// `uid` had made it (restic records the user running `backup`, which a test cannot change).
/// Returns the new snapshot ID: a snapshot file is named by the SHA-256 of its contents.
pub fn set_snapshot_owner(
    restic: &Path,
    repo: &Path,
    password: &str,
    snapshot_id: &str,
    owner: (u32, &str),
) -> String {
    let out = Command::new(restic)
        .arg("-r")
        .arg(repo)
        .args(["cat", "--no-cache", "snapshot", snapshot_id])
        .env("RESTIC_PASSWORD", password)
        .output()
        .expect("run restic cat snapshot");
    assert!(out.status.success(), "restic cat snapshot {snapshot_id}");
    let mut snapshot: Value = serde_json::from_slice(&out.stdout).expect("snapshot json");
    snapshot["uid"] = Value::from(owner.0);
    snapshot["username"] = Value::from(owner.1);
    let (_, key_json) = read_single_key(repo);
    let keys = Keys::from_master_json(&open_key(&key_json, password));
    // restic reads an unpacked file that starts with `{` as uncompressed JSON.
    let sealed = keys.seal(&serde_json::to_vec(&snapshot).expect("snapshot json"));
    let new_id = hex(&Sha256::digest(&sealed));
    let dir = repo.join("snapshots");
    write_read_only(&dir.join(&new_id), &sealed);
    fs::remove_file(dir.join(snapshot_id)).expect("remove old snapshot file");
    new_id
}

/// Encryption and MAC keys as restic uses them: AES-256 key plus Poly1305-AES (`k`, `r`).
struct Keys {
    encrypt: Vec<u8>,
    mac_k: Vec<u8>,
    mac_r: Vec<u8>,
}

impl Keys {
    /// Keys derived from a password with scrypt, split like restic: 32 + 16 + 16 bytes.
    fn derive(password: &str, salt: &[u8], (log_n, r, p): (u8, u32, u32)) -> Self {
        let params = scrypt::Params::new(log_n, r, p).expect("scrypt params");
        let mut out = [0u8; 64];
        scrypt::scrypt(password.as_bytes(), salt, &params, &mut out).expect("scrypt");
        Self {
            encrypt: out[..32].to_vec(),
            mac_k: out[32..48].to_vec(),
            mac_r: out[48..].to_vec(),
        }
    }

    /// The master key from its decrypted JSON (`{"mac":{"k","r"},"encrypt"}`).
    fn from_master_json(master: &Value) -> Self {
        let field = |v: &Value| {
            BASE64_STANDARD
                .decode(v.as_str().expect("string"))
                .expect("b64")
        };
        Self {
            encrypt: field(&master["encrypt"]),
            mac_k: field(&master["mac"]["k"]),
            mac_r: field(&master["mac"]["r"]),
        }
    }

    /// Poly1305-AES over `ciphertext`: key `r || AES-128_k(iv)`.
    fn mac(&self, iv: &[u8], ciphertext: &[u8]) -> Vec<u8> {
        let mut s = aes::Block::try_from(iv).expect("16-byte iv");
        aes::Aes128::new_from_slice(&self.mac_k)
            .expect("mac k")
            .encrypt_block(&mut s);
        let mut key = self.mac_r.clone();
        key.extend_from_slice(&s);
        poly1305::Poly1305::new_from_slice(&key)
            .expect("poly1305 key")
            .compute_unpadded(ciphertext)
            .to_vec()
    }

    fn apply_ctr(&self, iv: &[u8], data: &mut [u8]) {
        ctr::Ctr128BE::<aes::Aes256>::new_from_slices(&self.encrypt, iv)
            .expect("aes-256-ctr")
            .apply_keystream(data);
    }

    /// Decrypts restic's `iv || ciphertext || mac`; panics if the MAC does not match.
    fn open(&self, blob: &[u8], what: &str) -> Vec<u8> {
        assert!(blob.len() >= 32, "{what}: ciphertext too short");
        let (iv, rest) = blob.split_at(16);
        let (ciphertext, tag) = rest.split_at(rest.len() - 16);
        assert!(
            self.mac(iv, ciphertext) == tag,
            "{what}: MAC mismatch (wrong password or changed restic format)"
        );
        let mut plain = ciphertext.to_vec();
        self.apply_ctr(iv, &mut plain);
        plain
    }

    /// Encrypts `plain` as restic's `iv || ciphertext || mac` with a fresh IV.
    fn seal(&self, plain: &[u8]) -> Vec<u8> {
        let iv = random_bytes(16);
        let mut ciphertext = plain.to_vec();
        self.apply_ctr(&iv, &mut ciphertext);
        let tag = self.mac(&iv, &ciphertext);
        [iv, ciphertext, tag].concat()
    }
}

/// Path and contents of the only key file in `repo/keys`.
fn read_single_key(repo: &Path) -> (PathBuf, Value) {
    let keys: Vec<PathBuf> = fs::read_dir(repo.join("keys"))
        .expect("read keys dir")
        .map(|e| e.expect("key entry").path())
        .collect();
    assert_eq!(keys.len(), 1, "template must hold exactly one key");
    let json: Value =
        serde_json::from_slice(&fs::read(&keys[0]).expect("read key")).expect("key json");
    assert_eq!(
        json["kdf"], "scrypt",
        "restic key KDF changed; update repo_template.rs"
    );
    (keys[0].clone(), json)
}

/// The decrypted master key JSON of `key` (a restic key file).
fn open_key(key: &Value, password: &str) -> Value {
    let number = |name: &str| key[name].as_u64().expect("kdf parameter");
    let n = number("N");
    assert!(n.is_power_of_two(), "scrypt N must be a power of two");
    let params = (
        u8::try_from(n.trailing_zeros()).expect("log2 N"),
        u32::try_from(number("r")).expect("r"),
        u32::try_from(number("p")).expect("p"),
    );
    let salt = BASE64_STANDARD
        .decode(key["salt"].as_str().expect("salt"))
        .expect("salt b64");
    let data = BASE64_STANDARD
        .decode(key["data"].as_str().expect("data"))
        .expect("data b64");
    let plain = Keys::derive(password, &salt, params).open(&data, "key file");
    serde_json::from_slice(&plain).expect("master key json")
}

/// Writes a new key file for `master` protected by `password` with [`TEST_KDF`], named by the
/// SHA-256 of its contents like every restic key file.
fn write_key(repo: &Path, template_key: &Value, master: &Value, password: &str) {
    let salt = random_bytes(64);
    let plain = serde_json::to_vec(master).expect("master json");
    let data = Keys::derive(password, &salt, TEST_KDF).seal(&plain);
    let (log_n, r, p) = TEST_KDF;
    let mut key = template_key.clone();
    key["N"] = Value::from(1u64 << log_n);
    key["r"] = Value::from(r);
    key["p"] = Value::from(p);
    key["salt"] = Value::from(BASE64_STANDARD.encode(&salt));
    key["data"] = Value::from(BASE64_STANDARD.encode(&data));
    let raw = serde_json::to_vec(&key).expect("key json");
    write_read_only(&repo.join("keys").join(hex(&Sha256::digest(&raw))), &raw);
}

/// Re-encrypts the repository `config` with `repo_id` as its ID.
fn rewrite_config_id(repo: &Path, master: &Value, repo_id: &str) {
    let keys = Keys::from_master_json(master);
    let path = repo.join("config");
    let mut config: Value =
        serde_json::from_slice(&keys.open(&fs::read(&path).expect("read config"), "config"))
            .expect("config json");
    config["id"] = Value::from(repo_id);
    let sealed = keys.seal(&serde_json::to_vec(&config).expect("config json"));
    fs::remove_file(&path).expect("remove old config");
    write_read_only(&path, &sealed);
}

fn write_read_only(path: &Path, contents: &[u8]) {
    fs::write(path, contents).expect("write repository file");
    fs::set_permissions(path, fs::Permissions::from_mode(0o400)).expect("chmod 0400");
}

/// Copies `src` recursively into `dst`, keeping restic's owner-only directory permissions.
fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("create destination dir");
    let mode = fs::metadata(src)
        .expect("template dir metadata")
        .permissions();
    fs::set_permissions(dst, mode).expect("copy dir permissions");
    for entry in fs::read_dir(src).expect("read template dir") {
        let entry = entry.expect("template entry");
        let target = dst.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy template file");
        }
    }
}

/// `x.y.z` from `restic version`.
fn restic_version(restic: &Path) -> String {
    let out = Command::new(restic)
        .arg("version")
        .output()
        .expect("run restic version");
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace()
        .nth(1)
        .filter(|v| v.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .unwrap_or_else(|| panic!("unexpected `restic version` output: {text}"))
        .to_string()
}

/// Directory of the current test run's templates: `KESTIC_TEST_FIXTURE_RUN` (a fresh id per
/// `just test`/`just verify` run) or `local` for plain `cargo test`, where the template is reused
/// until `TEMPLATE_FORMAT` changes or `target/` is cleaned.
fn fixtures_run_dir() -> PathBuf {
    let run = env::var("KESTIC_TEST_FIXTURE_RUN")
        .ok()
        .filter(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or_else(|| "local".to_string());
    find_target_dir()
        .join("kestic-test-fixtures")
        .join(run)
        .join(format!("v{TEMPLATE_FORMAT}"))
}

/// The template repository and its password for `restic`'s version, created on first use.
fn template_repo(restic: &Path) -> (PathBuf, String) {
    template_in(&fixtures_run_dir().join(restic_version(restic)), restic)
}

/// The template repository in `dir` and its password, created there on first use.
fn template_in(dir: &Path, restic: &Path) -> (PathBuf, String) {
    if !dir.join("ready").is_file() {
        create_template(restic, dir);
    }
    let password = fs::read_to_string(dir.join("password")).expect("read template password");
    (dir.join("repo"), password)
}

/// Runs restic with an isolated environment (home, caches and config inside `home`).
fn run_restic(restic: &Path, home: &Path, password_file: &Path, args: &[&str]) -> Output {
    let path = env::var_os("PATH").unwrap_or_default();
    Command::new(restic)
        .env_clear()
        .env("PATH", path)
        .env("HOME", home)
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("RESTIC_CACHE_DIR", home.join("cache"))
        .arg("--password-file")
        .arg(password_file)
        .args(args)
        .output()
        .expect("run restic")
}

/// Builds the template in a private staging directory next to `dir` and moves it into place
/// atomically. A concurrent test process that loses the race discards its own staging copy.
fn create_template(restic: &Path, dir: &Path) {
    let parent = dir.parent().expect("template parent");
    fs::create_dir_all(parent).expect("create fixtures dir");
    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(parent)
        .expect("create staging dir");
    let root = staging.path();
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).expect("chmod staging");

    let password = strong_password();
    let password_file = root.join("password");
    fs::write(&password_file, &password).expect("write template password");
    fs::set_permissions(&password_file, fs::Permissions::from_mode(0o600)).expect("chmod 0600");

    let repo = root.join("repo");
    let repo_arg = repo.to_str().expect("utf-8 path");
    let init = run_restic(restic, root, &password_file, &["-r", repo_arg, "init"]);
    assert!(
        init.status.success(),
        "restic init for the test template failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let (key_path, key_json) = read_single_key(&repo);
    let master = open_key(&key_json, &password);
    write_key(&repo, &key_json, &master, &password);
    fs::remove_file(key_path).expect("remove expensive key");

    let check = run_restic(
        restic,
        root,
        &password_file,
        &["-r", repo_arg, "--no-lock", "cat", "config"],
    );
    assert!(
        check.status.success(),
        "restic rejects the re-wrapped template key: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    fs::remove_dir_all(root.join("cache")).ok();
    fs::write(root.join("ready"), b"").expect("write ready marker");

    let built = staging.keep();
    if fs::rename(&built, dir).is_err() {
        fs::remove_dir_all(&built).expect("discard losing staging copy");
        assert!(
            dir.join("ready").is_file(),
            "template {} missing after concurrent creation",
            dir.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::resolve_restic_binaries;

    /// restic's exit code for a wrong repository password.
    const EXIT_WRONG_PASSWORD: i32 = 12;

    /// Runs restic without lock against `repo` with `password` in an isolated home.
    fn restic_on(restic: &Path, repo: &TestRepo, password: &str, args: &[&str]) -> Output {
        let home = tempfile::tempdir().expect("home");
        let pw_file = home.path().join("pw");
        fs::write(&pw_file, password).expect("write pw");
        let mut full = vec!["-r", repo.path.to_str().expect("utf-8 path"), "--no-lock"];
        full.extend_from_slice(args);
        run_restic(restic, home.path(), &pw_file, &full)
    }

    fn config_id(restic: &Path, repo: &TestRepo) -> Value {
        let out = restic_on(restic, repo, &repo.password, &["cat", "config"]);
        assert!(out.status.success(), "copy does not open with its password");
        serde_json::from_slice::<Value>(&out.stdout).expect("config json")["id"].clone()
    }

    #[test]
    fn template_copies_open_with_own_password_and_id_for_each_restic_version() {
        for (label, restic) in resolve_restic_binaries() {
            let tmp = tempfile::tempdir().expect("tmp");
            let a = copy_template_repo(&restic, &tmp.path().join("a"));
            let b = copy_template_repo(&restic, &tmp.path().join("b"));

            // (a) each copy opens with its own password and reports its own repository ID
            assert_eq!(
                config_id(&restic, &a),
                a.repo_id.as_str(),
                "{label}: id of a"
            );
            assert_eq!(
                config_id(&restic, &b),
                b.repo_id.as_str(),
                "{label}: id of b"
            );
            assert_ne!(a.repo_id, b.repo_id, "{label}: repository IDs differ");

            // (b) a random password and the other copy's password are both rejected
            let wrong = restic_on(&restic, &a, &strong_password(), &["cat", "config"]);
            assert_eq!(
                wrong.status.code(),
                Some(EXIT_WRONG_PASSWORD),
                "{label}: random pw"
            );
            let crossed = restic_on(&restic, &b, &a.password, &["cat", "config"]);
            assert_eq!(
                crossed.status.code(),
                Some(EXIT_WRONG_PASSWORD),
                "{label}: a's pw on b"
            );

            // (c) the copy's key uses the cheap test KDF
            let (_, key) = read_single_key(&a.path);
            assert_eq!(key["N"], 1u64 << TEST_KDF.0, "{label}: scrypt N");

            // (d) restic can back up into a copy and the repository checks clean
            let src = tmp.path().join("src");
            fs::create_dir_all(&src).expect("src");
            fs::write(src.join("f"), b"data").expect("file");
            let src_arg = src.to_str().expect("src");
            let backup = restic_on(&restic, &b, &b.password, &["backup", src_arg]);
            assert!(backup.status.success(), "{label}: backup into copy");
            let check = restic_on(&restic, &b, &b.password, &["check"]);
            assert!(check.status.success(), "{label}: check of copy");
        }
    }

    #[test]
    fn concurrent_first_use_builds_one_template_and_leaves_no_staging() {
        let (_, restic) = resolve_restic_binaries().pop().expect("restic");
        let parent = tempfile::tempdir().expect("fixtures parent");
        let dir = parent.path().join("version");
        let passwords: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| scope.spawn(|| template_in(&dir, &restic).1))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("template thread"))
                .collect()
        });
        // (a) every caller sees the same template password
        assert!(passwords.iter().all(|p| *p == passwords[0]));
        // (b) only the finished template remains, no staging directory
        let entries: Vec<String> = fs::read_dir(parent.path())
            .expect("read parent")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["version".to_string()]);
    }
}
