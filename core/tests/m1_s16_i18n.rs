// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! Integration tests for M1-S16 (AC-M1-16-1, AC-M1-16-2: full EN/DE localization in GUI & daemon)
//! and NFR-06 (GNU gettext string extraction & Swiss-German catalog lint across all M1 sources).

use kestic_core::config::{AppConfig, save_config};
use kestic_core::model::ResticProvider;
use kestic_core::secrets::{FileSecretStore, SecretStore, SecretValue};
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct PrivateDbusGuard {
    address: String,
    pid: u32,
}

impl PrivateDbusGuard {
    fn start() -> Self {
        let out = Command::new("dbus-daemon")
            .arg(format!(
                "--config-file={}",
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../daemon/tests/fixtures/private-session-bus.xml"
                )
            ))
            .args(["--fork", "--print-address=1", "--print-pid=1"])
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .expect("spawn private dbus-daemon");
        assert!(
            out.status.success(),
            "dbus-daemon failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).expect("utf8 dbus-daemon output");
        let mut lines = stdout.lines();
        let address = lines.next().expect("dbus address").trim().to_string();
        let pid = lines
            .next()
            .expect("dbus pid")
            .trim()
            .parse::<u32>()
            .expect("parse dbus pid");
        Self { address, pid }
    }
}

impl Drop for PrivateDbusGuard {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &self.pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core crate has workspace root parent")
}

fn find_target_dir(root: &Path) -> PathBuf {
    env::var_os("CARGO_TARGET_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"))
}

fn find_kestic_binary(root: &Path) -> PathBuf {
    let target_dir = find_target_dir(root);
    for profile in ["debug", "release"] {
        let candidate = target_dir.join(profile).join("kestic");
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!("kestic binary not found under {}", target_dir.display());
}

fn collect_files_with_ext_rec(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read source dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_with_ext_rec(&path, ext, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some(ext) {
            out.push(path);
        }
    }
}

fn collect_files_with_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_files_with_ext_rec(dir, ext, &mut files);
    files.sort();
    files
}

/// NFR-06 (M1-S16): Extracts every `i18n(...)` string from all `daemon/src/*.rs`,
/// `gui/src/*.rs`, and `gui/src/qml/*.qml` files, plus every `i18n_noop(...)` msgid of the
/// shared texts in `i18n/src/*.rs`, and enforces 100% Swiss-German coverage
/// (0 untranslated, 0 fuzzy, 0 missing, 0 `ß`) in `po/de/kestic.po`.
#[test]
fn nfr_06_m1_all_rust_and_qml_i18n_strings_extracted_and_translated() {
    let root = workspace_root();
    let po_file = root.join("po/de/kestic.po");

    let mut rust_files = collect_files_with_ext(&root.join("daemon/src"), "rs");
    rust_files.extend(collect_files_with_ext(&root.join("gui/src"), "rs"));
    // The shared texts of `kestic-i18n` are marked with `i18n_noop`.
    rust_files.extend(collect_files_with_ext(&root.join("i18n/src"), "rs"));
    let qml_files = collect_files_with_ext(&root.join("gui/src/qml"), "qml");
    assert!(!rust_files.is_empty(), "expected Rust files");
    assert!(!qml_files.is_empty(), "expected QML files");

    let temp = root.join(format!(
        "target/test-tmp-m1-s16-lint-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).expect("create temp pot dir");
    let _guard = TempDirGuard(temp.clone());
    let pot_file = temp.join("kestic.pot");

    let rust_out = Command::new("xgettext")
        .args([
            "--from-code=UTF-8",
            "--language=C",
            "--keyword=i18n:1",
            "--keyword=i18n_noop:1",
            "-o",
        ])
        .arg(&pot_file)
        .args(&rust_files)
        .output()
        .expect("run xgettext on M1 Rust sources");
    assert!(
        rust_out.status.success(),
        "xgettext (Rust) failed:\n{}",
        String::from_utf8_lossy(&rust_out.stderr)
    );

    let qml_out = Command::new("xgettext")
        .args([
            "--from-code=UTF-8",
            "--language=JavaScript",
            "--keyword=i18n:1",
            "-j",
            "-o",
        ])
        .arg(&pot_file)
        .args(&qml_files)
        .output()
        .expect("run xgettext on M1 QML sources");
    assert!(
        qml_out.status.success(),
        "xgettext (QML) failed:\n{}",
        String::from_utf8_lossy(&qml_out.stderr)
    );

    let cmp_out = Command::new("msgcmp")
        .arg(&po_file)
        .arg(&pot_file)
        .output()
        .expect("run msgcmp on po/de/kestic.po");
    assert!(
        cmp_out.status.success(),
        "msgcmp found missing or untranslated M1 msgids in {}:\n{}",
        po_file.display(),
        String::from_utf8_lossy(&cmp_out.stderr)
    );

    let untranslated = Command::new("msgattrib")
        .arg("--untranslated")
        .arg(&po_file)
        .output()
        .expect("run msgattrib --untranslated");
    assert!(untranslated.status.success(), "msgattrib failed");
    assert!(
        String::from_utf8_lossy(&untranslated.stdout)
            .trim()
            .is_empty(),
        "po/de/kestic.po still has untranslated entries:\n{}",
        String::from_utf8_lossy(&untranslated.stdout)
    );

    let fuzzy = Command::new("msgattrib")
        .arg("--only-fuzzy")
        .arg(&po_file)
        .output()
        .expect("run msgattrib --only-fuzzy");
    assert!(fuzzy.status.success(), "msgattrib --only-fuzzy failed");
    assert!(
        String::from_utf8_lossy(&fuzzy.stdout).trim().is_empty(),
        "po/de/kestic.po still has fuzzy entries:\n{}",
        String::from_utf8_lossy(&fuzzy.stdout)
    );

    let msgfmt_out = Command::new("msgfmt")
        .args(["--check", "-o", "/dev/null"])
        .arg(&po_file)
        .output()
        .expect("run msgfmt --check");
    assert!(
        msgfmt_out.status.success(),
        "msgfmt --check failed:\n{}",
        String::from_utf8_lossy(&msgfmt_out.stderr)
    );

    let po_text = fs::read_to_string(&po_file).expect("read po/de/kestic.po");
    assert!(
        !po_text.contains('ß'),
        "po/de/kestic.po must follow Swiss German orthography (no `ß`, use `ss`)"
    );
}

fn run_lang_smoke(lang: &str, screenshot_rel: &str) {
    let root = workspace_root();
    let kestic = find_kestic_binary(root);
    let built_mo = find_target_dir(root).join("locale/de/LC_MESSAGES/kestic.mo");
    assert!(
        built_mo.is_file() && fs::metadata(&built_mo).expect("stat kestic.mo").len() > 0,
        "compiled German catalog missing at {}",
        built_mo.display()
    );

    let temp = root.join(format!(
        "target/test-tmp-m1-s16-{lang}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp);
    for sub in [
        "home",
        "cfg/kestic",
        "data/locale/de/LC_MESSAGES",
        "cache",
        "state/kestic",
        "run",
    ] {
        fs::create_dir_all(temp.join(sub)).expect("mkdir isolated");
    }
    fs::set_permissions(temp.join("run"), fs::Permissions::from_mode(0o700))
        .expect("chmod 0700 run");
    fs::copy(&built_mo, temp.join("data/locale/de/LC_MESSAGES/kestic.mo")).expect("copy kestic.mo");
    let _guard = TempDirGuard(temp.clone());

    let secret_file = temp.join("secrets.hex");
    FileSecretStore::new(&secret_file)
        .set_secret(
            "kestic_health_probe",
            "probe",
            &SecretValue::from_str_secret("ok"),
        )
        .expect("seed probe");

    let cfg_path = temp.join("cfg/kestic/config.toml");
    let cfg = AppConfig {
        restic_provider: ResticProvider::Custom {
            path: PathBuf::from("/nonexistent/kestic-missing-restic"),
        },
        ..AppConfig::default()
    };
    save_config(&cfg_path, &cfg).expect("save config");

    let dbus = PrivateDbusGuard::start();
    let ready_file = temp.join("daemon.ready");
    let daemon_child = Command::new(&kestic)
        .args([
            "daemon",
            "--no-job-view",
            "--no-portal-inhibit",
            "--suppress-ui-spawn",
            "--config-path",
        ])
        .arg(&cfg_path)
        .arg("--state-dir")
        .arg(temp.join("state/kestic"))
        .arg("--ready-file")
        .arg(&ready_file)
        .env("HOME", temp.join("home"))
        .env("XDG_CONFIG_HOME", temp.join("cfg"))
        .env("XDG_DATA_HOME", temp.join("data"))
        .env("XDG_CACHE_HOME", temp.join("cache"))
        .env("XDG_STATE_HOME", temp.join("state"))
        .env("XDG_RUNTIME_DIR", temp.join("run"))
        .env("DBUS_SESSION_BUS_ADDRESS", &dbus.address)
        .env("KESTIC_SECRET_FILE", &secret_file)
        .env_remove("LC_ALL")
        .env_remove("LC_MESSAGES")
        .env("LANG", "en_US.UTF-8")
        .env("LANGUAGE", lang)
        .env("QT_QPA_PLATFORM", "offscreen")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon");
    let _daemon_guard = ChildGuard(daemon_child);

    let deadline = Instant::now() + Duration::from_secs(8);
    while !ready_file.is_file() {
        assert!(Instant::now() < deadline, "daemon ready timeout");
        thread::sleep(Duration::from_millis(25));
    }

    let screenshot = root.join(screenshot_rel);
    if screenshot.exists() {
        fs::remove_file(&screenshot).expect("remove stale screenshot");
    }

    let out = Command::new(&kestic)
        .args([
            "--smoke-m1",
            "blocking-restic_unavailable",
            "--smoke-layout",
            "wide",
            "--smoke-lang",
            lang,
            "--screenshot",
        ])
        .arg(&screenshot)
        .env("HOME", temp.join("home"))
        .env("XDG_CONFIG_HOME", temp.join("cfg"))
        .env("XDG_DATA_HOME", temp.join("data"))
        .env("XDG_CACHE_HOME", temp.join("cache"))
        .env("XDG_STATE_HOME", temp.join("state"))
        .env("XDG_RUNTIME_DIR", temp.join("run"))
        .env("DBUS_SESSION_BUS_ADDRESS", &dbus.address)
        .env("KESTIC_SECRET_FILE", &secret_file)
        .env_remove("LC_ALL")
        .env_remove("LC_MESSAGES")
        .env("LANG", "en_US.UTF-8")
        .env("LANGUAGE", lang)
        .env("QT_QPA_PLATFORM", "offscreen")
        .output()
        .expect("run GUI smoke for M1-S16");
    assert!(
        out.status.success(),
        "M1-S16 `{lang}` smoke failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).trim().is_empty(),
        "M1-S16 `{lang}` emitted QML warnings:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let png = fs::read(&screenshot).expect("read screenshot");
    assert!(
        png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 2048,
        "M1-S16 screenshot {} must be valid PNG",
        screenshot.display()
    );
}

/// AC-M1-16-1 & AC-M1-16-2: Verifies both GUI and daemon KI18n translation in EN and DE.
#[test]
fn ac_m1_16_1_and_16_2_ki18n_en_and_de_in_gui_and_daemon() {
    run_lang_smoke("en", "target/smoke-artifacts/m1-s16-en.png");
    run_lang_smoke("de", "target/smoke-artifacts/m1-s16-de.png");
}
