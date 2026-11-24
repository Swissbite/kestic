// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::restic_install::{InstallRequest, prepare_with};
use std::collections::HashMap;
use std::sync::Mutex;

/// Transport that answers from a table and records every requested URL.
#[derive(Default)]
struct RecordingTransport {
    answers: HashMap<String, HttpAnswer>,
    requests: Mutex<Vec<String>>,
}

impl RecordingTransport {
    fn ok(mut self, url: &str, body: &[u8]) -> Self {
        self.answers.insert(
            url.to_string(),
            HttpAnswer {
                status: 200,
                location: None,
                body: body.to_vec(),
            },
        );
        self
    }

    fn redirect(mut self, url: &str, location: &str) -> Self {
        self.answers.insert(
            url.to_string(),
            HttpAnswer {
                status: 302,
                location: Some(location.to_string()),
                body: Vec::new(),
            },
        );
        self
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("request log").clone()
    }
}

impl Transport for RecordingTransport {
    fn get(&self, url: &Url, limit: u64) -> Result<HttpAnswer, InstallError> {
        self.requests
            .lock()
            .expect("request log")
            .push(url.to_string());
        let answer = self
            .answers
            .get(url.as_str())
            .cloned()
            .unwrap_or(HttpAnswer {
                status: 404,
                location: None,
                body: Vec::new(),
            });
        if answer.body.len() as u64 > limit {
            return Err(InstallError::Network(format!("larger than {limit} bytes")));
        }
        Ok(answer)
    }
}

fn latest_json(tag: &str) -> Vec<u8> {
    format!(
        "{{\"tag_name\":\"{tag}\",\"draft\":false,\"prerelease\":false,\"assets\":[{{\"name\":\"SHA256SUMS\",\"browser_download_url\":\"https://downloads.example.invalid/SHA256SUMS\"}}]}}"
    )
    .into_bytes()
}

const LATEST: &str = "https://api.github.com/repos/restic/restic/releases/latest";
const SUMS_0_19_1: &str = "https://github.com/restic/restic/releases/download/v0.19.1/SHA256SUMS";

/// AC-M5-06-2: on arm64 the plan picks the arm64 archive.
#[test]
fn ac_m5_06_2_arm64_plan_picks_arm64_asset() {
    let anchor = TrustAnchor::production();
    let version = Version::new(0, 19, 1);
    let arm = DownloadPlan::new(
        &anchor,
        &version,
        Arch::from_rust("aarch64").expect("arm64"),
    );
    // (a)
    assert_eq!(arm.archive_name, "restic_0.19.1_linux_arm64.bz2");
    assert_eq!(
        arm.archive_url,
        "https://github.com/restic/restic/releases/download/v0.19.1/restic_0.19.1_linux_arm64.bz2"
    );
    assert_eq!(arm.sums_url, SUMS_0_19_1);
    assert_eq!(
        arm.signature_url,
        "https://github.com/restic/restic/releases/download/v0.19.1/SHA256SUMS.asc"
    );
    let amd = DownloadPlan::new(&anchor, &version, Arch::from_rust("x86_64").expect("amd64"));
    assert_eq!(amd.archive_name, "restic_0.19.1_linux_amd64.bz2");
    assert_eq!(
        Arch::from_rust("riscv64"),
        Err(InstallError::UnsupportedArchitecture("riscv64".to_string()))
    );
}

/// AC-M5-06-3: the production build carries restic's signing key and its fingerprint.
#[test]
fn ac_m5_06_3_production_fingerprint() {
    let anchor = TrustAnchor::production();
    // (a)
    assert_eq!(
        anchor.fingerprint_grouped(),
        "CF8F 18F2 8445 7597 3F79 D4E1 91A6 868B D3F7 A907"
    );
    assert_eq!(
        anchor.fingerprint(),
        "CF8F18F2844575973F79D4E191A6868BD3F7A907"
    );

    // The embedded key is the key with this fingerprint.
    let home = tempfile::tempdir().expect("GNUPGHOME");
    let key_file = home.path().join("key.pgp");
    std::fs::write(&key_file, anchor.key()).expect("write key");
    let out = Command::new("gpg")
        .env("GNUPGHOME", home.path())
        .args(["--batch", "--with-colons", "--show-keys"])
        .arg(&key_file)
        .output()
        .expect("run gpg --show-keys");
    let listing = String::from_utf8_lossy(&out.stdout);
    let primary_fpr = listing
        .lines()
        .skip_while(|l| !l.starts_with("pub:"))
        .find_map(|l| l.strip_prefix("fpr:").and_then(|r| r.split(':').nth(8)))
        .expect("primary key fingerprint");
    assert_eq!(primary_fpr, "CF8F18F2844575973F79D4E191A6868BD3F7A907");
}

/// AC-M5-06-3: the embedded key verifies a real restic release signature through `gpgv`, and
/// the export holds exactly one primary key.
#[test]
fn ac_m5_06_3_embedded_key_verifies_real_release() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m5");
    let sums = std::fs::read(fixtures.join("restic_0.20.0_SHA256SUMS")).expect("SHA256SUMS");
    let signature =
        std::fs::read(fixtures.join("restic_0.20.0_SHA256SUMS.asc")).expect("SHA256SUMS.asc");
    let work = tempfile::tempdir().expect("work dir");
    let gpgv = find_gpgv(std::env::var_os("PATH").as_deref()).expect("gpgv in PATH");
    let anchor = TrustAnchor::production();
    // (a)
    assert_eq!(
        verify_signature(&gpgv, &anchor, work.path(), &sums, &signature),
        Ok(())
    );
    let mut tampered = sums.clone();
    tampered[0] ^= 1;
    assert!(matches!(
        verify_signature(&gpgv, &anchor, work.path(), &tampered, &signature),
        Err(InstallError::SignatureInvalid(_))
    ));
    assert!(
        expected_sha256(
            &String::from_utf8_lossy(&sums),
            "restic_0.20.0_linux_amd64.bz2"
        )
        .is_ok()
    );
    assert_eq!(
        std::fs::read_dir(work.path()).expect("work dir").count(),
        0,
        "gpgv working folder left behind"
    );

    let home = tempfile::tempdir().expect("GNUPGHOME");
    let key_file = home.path().join("key.pgp");
    std::fs::write(&key_file, anchor.key()).expect("write key");
    let out = Command::new("gpg")
        .env("GNUPGHOME", home.path())
        .args(["--batch", "--with-colons", "--show-keys"])
        .arg(&key_file)
        .output()
        .expect("run gpg --show-keys");
    let listing = String::from_utf8_lossy(&out.stdout);
    assert_eq!(listing.lines().filter(|l| l.starts_with("pub:")).count(), 1);
}

const FPR: &str = "CF8F18F2844575973F79D4E191A6868BD3F7A907";

/// AC-M5-06-4: only a `VALIDSIG` of restic's primary key without any bad status line counts.
#[test]
fn ac_m5_06_4_status_lines() {
    let valid = format!(
        "[GNUPG:] NEWSIG\n[GNUPG:] GOODSIG 91A6868BD3F7A907 Alexander Neumann\n\
         [GNUPG:] VALIDSIG {FPR} 2026-11-19 1795087821 0 4 0 1 10 00 {FPR}\n"
    );
    assert_eq!(check_gpgv_status(&valid, true, FPR), Ok(()));
    // A subkey signature names the primary key in the last field.
    let subkey = format!(
        "[GNUPG:] VALIDSIG 07C478580126F4E4006FABA1D5FC2ACF4043FDF1 2026-11-19 1 0 4 0 1 10 00 {FPR}\n"
    );
    assert_eq!(check_gpgv_status(&subkey, true, FPR), Ok(()));

    let other = "[GNUPG:] VALIDSIG 1111111111111111111111111111111111111111 2026-11-19 1 0 4 0 22 10 00 1111111111111111111111111111111111111111\n";
    assert!(check_gpgv_status(other, true, FPR).is_err());
    // The signing key is restic's, but its primary key is another one.
    let foreign_primary = format!(
        "[GNUPG:] VALIDSIG {FPR} 2026-11-19 1 0 4 0 1 10 00 1111111111111111111111111111111111111111\n"
    );
    assert!(check_gpgv_status(&foreign_primary, true, FPR).is_err());
    assert!(check_gpgv_status(&valid, false, FPR).is_err());
    assert!(check_gpgv_status("", true, FPR).is_err());
    for bad in ["BADSIG", "EXPKEYSIG", "REVKEYSIG", "ERRSIG"] {
        let status = format!("{valid}[GNUPG:] {bad} 91A6868BD3F7A907 x\n");
        assert!(
            check_gpgv_status(&status, true, FPR).is_err(),
            "{bad} must make the check fail"
        );
    }
    // The fingerprint only counts as the line's last field, and as a whole.
    let embedded = format!("[GNUPG:] VALIDSIG {FPR} 2026-11-19 1 0 4 0 1 10 00 {FPR}0\n");
    assert!(check_gpgv_status(&embedded, true, FPR).is_err());
    let not_status = format!("VALIDSIG {FPR} 2026-11-19 1 0 4 0 1 10 00 {FPR}\n");
    assert!(check_gpgv_status(&not_status, true, FPR).is_err());
    let suffix = format!(
        "[GNUPG:] VALIDSIG {FPR} 2026-11-19 1 0 4 0 1 10 00 0{}\n",
        &FPR[1..]
    );
    assert!(check_gpgv_status(&suffix, true, FPR).is_err());
    let prefix = format!(
        "[GNUPG:] VALIDSIG {FPR} 2026-11-19 1 0 4 0 1 10 00 {}\n",
        &FPR[..39]
    );
    assert!(check_gpgv_status(&prefix, true, FPR).is_err());
    let truncated = format!("[GNUPG:] VALIDSIG {FPR} 2026-11-19 {FPR}\n");
    assert!(check_gpgv_status(&truncated, true, FPR).is_err());
    let expired = format!("{valid}[GNUPG:] EXPSIG 91A6868BD3F7A907 x\n");
    assert!(check_gpgv_status(&expired, true, FPR).is_err());
    let bad_first = format!("[GNUPG:] BADSIG 1111111111111111 x\n{valid}");
    assert!(check_gpgv_status(&bad_first, true, FPR).is_err());
}

/// AC-M5-06-4: the right subkey under a wrong primary key, two signatures, and `GOODSIG`
/// without `VALIDSIG` are each refused.
#[test]
fn ac_m5_06_4_status_lines_subkey_two_signatures_goodsig_only() {
    let subkey = "07C478580126F4E4006FABA1D5FC2ACF4043FDF1";
    let wrong_primary = format!(
        "[GNUPG:] NEWSIG\n[GNUPG:] GOODSIG D5FC2ACF4043FDF1 x\n\
         [GNUPG:] VALIDSIG {subkey} 2026-11-19 1 0 4 0 1 10 00 1111111111111111111111111111111111111111\n"
    );
    assert!(check_gpgv_status(&wrong_primary, true, FPR).is_err());

    let one = format!(
        "[GNUPG:] NEWSIG\n[GNUPG:] GOODSIG 91A6868BD3F7A907 x\n\
         [GNUPG:] VALIDSIG {FPR} 2026-11-19 1 0 4 0 1 10 00 {FPR}\n"
    );
    assert_eq!(check_gpgv_status(&one, true, FPR), Ok(()));
    let two = format!("{one}{one}");
    assert!(check_gpgv_status(&two, true, FPR).is_err());
    let two_validsig = format!(
        "[GNUPG:] VALIDSIG {FPR} 2026-11-19 1 0 4 0 1 10 00 {FPR}\n\
         [GNUPG:] VALIDSIG {FPR} 2026-11-19 1 0 4 0 1 10 00 {FPR}\n"
    );
    assert!(check_gpgv_status(&two_validsig, true, FPR).is_err());

    let goodsig_only = "[GNUPG:] NEWSIG\n[GNUPG:] GOODSIG 91A6868BD3F7A907 x\n";
    assert!(check_gpgv_status(goodsig_only, true, FPR).is_err());
}

/// The release build trusts only a `gpgv` that root owns and only root can change.
#[test]
fn only_a_root_owned_gpgv_is_trusted() {
    // (a) the system's gpgv, also through a symbolic link that resolves to it
    assert_eq!(
        trusted_gpgv(Path::new(SYSTEM_GPGV)),
        Some(PathBuf::from(SYSTEM_GPGV))
    );
    let dir = tempfile::tempdir().expect("dir");
    let link = dir.path().join("gpgv");
    std::os::unix::fs::symlink(SYSTEM_GPGV, &link).expect("symlink");
    assert_eq!(trusted_gpgv(&link), Some(PathBuf::from(SYSTEM_GPGV)));
    // (b) a gpgv of this user (tests never run as root) is refused
    let own = dir.path().join("own");
    std::fs::create_dir(&own).expect("own dir");
    let fake = own.join("gpgv");
    std::fs::copy(SYSTEM_GPGV, &fake).expect("copy gpgv");
    assert_eq!(trusted_gpgv(&fake), None);
    assert_eq!(trusted_gpgv(&dir.path().join("missing")), None);
    // (c) only absolute PATH entries count
    let search = std::env::join_paths([Path::new("relative"), own.as_path()]).expect("PATH");
    assert_eq!(gpgv_in_path(Some(&search)), Some(fake.clone()));
    assert_eq!(gpgv_in_path(Some(OsStr::new("relative"))), None);
    // (d) the release build's choice: the system gpgv first, then only a trusted one from PATH
    let missing = dir.path().join("no-system-gpgv");
    assert_eq!(
        select_trusted_gpgv(Path::new(SYSTEM_GPGV), Some(&search)),
        Some(PathBuf::from(SYSTEM_GPGV))
    );
    assert_eq!(select_trusted_gpgv(&missing, Some(&search)), None);
    let system_dir = std::env::join_paths([Path::new("/usr/bin")]).expect("PATH");
    assert_eq!(
        select_trusted_gpgv(&missing, Some(&system_dir)),
        Some(PathBuf::from(SYSTEM_GPGV))
    );
}

/// Invisible format and direction characters from the network cannot reorder or hide parts of
/// an error detail: they become spaces like control characters.
#[test]
fn clip_replaces_format_and_bidi_characters() {
    for c in [
        '\u{200B}',
        '\u{200C}',
        '\u{200D}',
        '\u{200E}',
        '\u{200F}',
        '\u{202A}',
        '\u{202B}',
        '\u{202C}',
        '\u{202D}',
        '\u{202E}',
        '\u{2066}',
        '\u{2067}',
        '\u{2068}',
        '\u{2069}',
        '\u{2028}',
        '\u{2029}',
        '\u{FEFF}',
        '\u{061C}',
        '\u{00AD}',
        '\u{034F}',
        '\u{115F}',
        '\u{1160}',
        '\u{180E}',
        '\u{2060}',
        '\u{2064}',
        '\u{3164}',
        '\u{FE00}',
        '\u{FE0F}',
        '\u{FFA0}',
        '\u{FFF9}',
        '\u{FFFB}',
        '\u{E0001}',
        '\u{E007F}',
        '\u{2800}',
    ] {
        assert_eq!(
            clip(&format!("a{c}b")),
            "a b",
            "U+{:04X} is replaced",
            u32::from(c)
        );
    }
    assert_eq!(
        clip("Prüfsumme ä ß €"),
        "Prüfsumme ä ß €",
        "visible text stays"
    );
}

/// Text from the network or from `gpgv` reaches an error detail only cut to its limit.
#[test]
fn attacker_text_is_clipped_in_error_details() {
    assert_eq!(clip("short"), "short");
    assert_eq!(clip("a\nb\u{1b}[31mc"), "a b [31mc");
    let long = "ä".repeat(DETAIL_LIMIT + 50);
    let clipped = clip(&long);
    assert_eq!(clipped.chars().count(), DETAIL_LIMIT + 1);
    assert!(clipped.ends_with('…'));

    let location = format!("https://evil.example/{}", "x".repeat(5000));
    let transport = RecordingTransport::default().redirect(SUMS_0_19_1, &location);
    let Err(InstallError::HostNotAllowed(detail)) = fetch_asset(
        &transport,
        &TrustAnchor::production(),
        SUMS_0_19_1,
        RELEASE_INFO_LIMIT,
        "SHA256SUMS",
    ) else {
        panic!("a foreign redirect must be refused");
    };
    assert!(detail.chars().count() <= DETAIL_LIMIT + 1, "{detail}");

    let tag = format!("v{}", "9".repeat(5000));
    let transport = RecordingTransport::default().ok(LATEST, &latest_json(&tag));
    let Err(InstallError::AssetMissing(detail)) =
        fetch_release(&transport, &TrustAnchor::production())
    else {
        panic!("an unusable tag must be refused");
    };
    assert!(detail.chars().count() < DETAIL_LIMIT + 50, "{detail}");
}

/// AC-M5-06-5: the checksum comes from exactly one line naming exactly the asset.
#[test]
fn ac_m5_06_5_sums_line_lookup() {
    let hex_a = "aa".repeat(32);
    let hex_b = "bb".repeat(32);
    let name = "restic_0.19.1_linux_amd64.bz2";
    let sums =
        format!("{hex_b}  x{name}\n{hex_a}  {name}\n{hex_b}  restic_0.19.1_linux_arm64.bz2\n");
    assert_eq!(expected_sha256(&sums, name), Ok([0xaa; 32]));
    assert_eq!(
        expected_sha256(&format!("{hex_a} *{name}\n"), name),
        Ok([0xaa; 32])
    );
    assert!(matches!(
        expected_sha256(&format!("{hex_b}  x{name}\n"), name),
        Err(InstallError::AssetMissing(_))
    ));
    assert!(matches!(
        expected_sha256(&format!("{hex_a}  {name}\n{hex_b}  {name}\n"), name),
        Err(InstallError::ChecksumMismatch(_))
    ));
    assert!(matches!(
        expected_sha256(&format!("{}  {name}\n", "zz".repeat(32)), name),
        Err(InstallError::AssetMissing(_))
    ));
    for short in ["", "aa", &"a".repeat(63), &"a".repeat(65)] {
        assert!(
            matches!(
                expected_sha256(&format!("{short}  {name}\n"), name),
                Err(InstallError::AssetMissing(_))
            ),
            "hash {short:?}"
        );
    }
    assert_eq!(
        expected_sha256(&format!("{}  {name}\r\n", "AA".repeat(32)), name),
        Ok([0xaa; 32])
    );

    let data = b"restic";
    let digest: [u8; 32] = Sha256::digest(data).into();
    assert_eq!(check_sha256(data, &digest, name), Ok(()));
    assert_eq!(
        check_sha256(b"tampered", &digest, name),
        Err(InstallError::ChecksumMismatch(name.to_string()))
    );
}

/// AC-M5-06-9: a release below restic 0.18.0 is refused before any of its files is fetched,
/// and so is a new binary that reports such a version.
#[test]
fn ac_m5_06_9_release_below_minimum_is_refused() {
    let transport = RecordingTransport::default().ok(LATEST, &latest_json("v0.17.1"));
    let data = tempfile::tempdir().expect("data dir");
    let path = std::env::var_os("PATH");
    let request = InstallRequest {
        data_dir: data.path(),
        arch: Arch::Amd64,
        search_path: path.as_deref(),
    };
    let result = prepare_with(&request, &TrustAnchor::production(), &transport);
    // (a)
    assert_eq!(
        result.map(|p| p.into_ready().map(|p| p.version().clone())),
        Err(InstallError::TooOld {
            found: "0.17.1".to_string(),
            min: "0.18.0".to_string()
        })
    );
    assert_eq!(transport.requests(), vec![LATEST.to_string()]);
    assert_eq!(
        crate::restic_install::check_version(&Version::new(0, 17, 1), &Version::new(0, 17, 1)),
        Err(InstallError::TooOld {
            found: "0.17.1".to_string(),
            min: "0.18.0".to_string()
        })
    );
    assert_eq!(
        crate::restic_install::check_version(&Version::new(0, 18, 0), &Version::new(0, 19, 1)),
        Err(InstallError::VersionMismatch {
            expected: "0.19.1".to_string(),
            found: "0.18.0".to_string()
        })
    );
    assert_eq!(
        crate::restic_install::check_version(&Version::new(0, 19, 1), &Version::new(0, 19, 1)),
        Ok(())
    );
    assert_eq!(
        crate::restic_install::check_version(&Version::new(0, 18, 0), &Version::new(0, 18, 0)),
        Ok(())
    );
    assert!(matches!(
        crate::restic_install::check_version(&Version::new(0, 17, 99), &Version::new(0, 17, 99)),
        Err(InstallError::TooOld { .. })
    ));
    let transport = RecordingTransport::default().ok(LATEST, &latest_json("v0.17.99"));
    assert!(matches!(
        fetch_release(&transport, &TrustAnchor::production()),
        Err(InstallError::TooOld { .. })
    ));
}

/// AC-M5-06-11: release information only from GitHub's API, assets only from restic's
/// download path, redirects only to GitHub's download hosts.
#[test]
fn ac_m5_06_11_release_hosts_and_redirect_allowlist() {
    let anchor = TrustAnchor::production();
    let transport = RecordingTransport::default().ok(LATEST, &latest_json("v0.19.1"));
    // (a)
    assert_eq!(
        fetch_release(&transport, &anchor),
        Ok(Version::new(0, 19, 1))
    );
    assert_eq!(transport.requests(), vec![LATEST.to_string()]);

    // (b) followed: https to githubusercontent.com subdomains; the JSON's URL is never used
    let objects = "https://objects.githubusercontent.com/release/1?sig=x";
    let assets = "https://release-assets.githubusercontent.com/github-production-release-asset/2";
    let transport = RecordingTransport::default()
        .redirect(SUMS_0_19_1, objects)
        .redirect(objects, assets)
        .ok(assets, b"sums");
    let plan = DownloadPlan::new(&anchor, &Version::new(0, 19, 1), Arch::Amd64);
    assert_eq!(
        fetch_asset(
            &transport,
            &anchor,
            &plan.sums_url,
            RELEASE_INFO_LIMIT,
            "SHA256SUMS"
        ),
        Ok(b"sums".to_vec())
    );
    assert_eq!(
        transport.requests(),
        vec![
            SUMS_0_19_1.to_string(),
            objects.to_string(),
            assets.to_string()
        ]
    );
    assert!(
        plan.archive_url
            .starts_with("https://github.com/restic/restic/releases/download/v0.19.1/")
    );

    // (c) any other target ends the download before it is requested
    for target in [
        "https://evilgithubusercontent.com/x",
        "https://githubusercontent.com.evil.example/x",
        "http://objects.githubusercontent.com/x",
        "https://user@evil.example/x",
        "https://objects.githubusercontent.com:8443/x",
        "https://140.82.112.3/x",
        "https://example.com/objects.githubusercontent.com",
        "https://github.com/evil/x",
        "/x",
        "//evil.example/x",
        "https://evil@objects.githubusercontent.com/x",
        "https://evil.example\\@objects.githubusercontent.com/x",
        "https://objects.githubusercontent.com./x",
        "https://githubusercontent.com/x",
        "https://a..githubusercontent.com/x",
        "https://..githubusercontent.com/x",
        "https://a_b.githubusercontent.com/x",
    ] {
        let transport = RecordingTransport::default()
            .redirect(SUMS_0_19_1, target)
            .ok(target, b"evil");
        let result = fetch_asset(
            &transport,
            &anchor,
            SUMS_0_19_1,
            RELEASE_INFO_LIMIT,
            "SHA256SUMS",
        );
        assert!(
            matches!(result, Err(InstallError::HostNotAllowed(_))),
            "redirect to {target}: {result:?}"
        );
        assert_eq!(
            transport.requests(),
            vec![SUMS_0_19_1.to_string()],
            "{target}"
        );
    }

    // at most five redirects
    let hop = |n: usize| format!("https://objects.githubusercontent.com/{n}");
    let mut transport = RecordingTransport::default().redirect(SUMS_0_19_1, &hop(0));
    for n in 0..6 {
        transport = transport.redirect(&hop(n), &hop(n + 1));
    }
    let result = fetch_asset(
        &transport,
        &anchor,
        SUMS_0_19_1,
        RELEASE_INFO_LIMIT,
        "SHA256SUMS",
    );
    assert!(
        matches!(result, Err(InstallError::Network(_))),
        "{result:?}"
    );
    assert_eq!(transport.requests().len(), MAX_REDIRECTS + 1);

    // an http hop in the middle of a chain stops it there
    let objects = "https://objects.githubusercontent.com/1";
    let plain = "http://objects.githubusercontent.com/2";
    let transport = RecordingTransport::default()
        .redirect(SUMS_0_19_1, objects)
        .redirect(objects, plain)
        .ok(plain, b"evil");
    assert!(matches!(
        fetch_asset(
            &transport,
            &anchor,
            SUMS_0_19_1,
            RELEASE_INFO_LIMIT,
            "SHA256SUMS"
        ),
        Err(InstallError::HostNotAllowed(_))
    ));
    assert_eq!(
        transport.requests(),
        vec![SUMS_0_19_1.to_string(), objects.to_string()]
    );

    // the release information follows the same rule
    let transport = RecordingTransport::default()
        .redirect(LATEST, "https://api.evil.example/latest")
        .ok("https://api.evil.example/latest", &latest_json("v0.19.1"));
    assert!(matches!(
        fetch_release(&transport, &anchor),
        Err(InstallError::HostNotAllowed(_))
    ));
    assert_eq!(transport.requests(), vec![LATEST.to_string()]);

    // a tag that is not v<major>.<minor>.<patch> names no release
    for tag in [
        "v0.19.1-rc1",
        "v0.19.1+x",
        "v0.19.01",
        "v0.19.1/../x",
        "v0.19.1\n",
        "v0.19.1.1",
        "0.19.1",
        "V0.19.1",
        "v0.19",
        "v99999999999999999999.0.0",
        "latest",
    ] {
        let transport = RecordingTransport::default().ok(LATEST, &latest_json(tag));
        assert!(
            matches!(
                fetch_release(&transport, &anchor),
                Err(InstallError::AssetMissing(_))
            ),
            "{tag}"
        );
    }
}

/// AC-M5-06-11: answers other than `200` and release information in another form stop the
/// download with a code, never a panic.
#[test]
fn ac_m5_06_11_http_and_json_failures() {
    let anchor = TrustAnchor::production();
    let archive =
        "https://github.com/restic/restic/releases/download/v0.19.1/restic_0.19.1_linux_amd64.bz2";
    let transport = RecordingTransport::default();
    assert!(matches!(
        fetch_asset(
            &transport,
            &anchor,
            archive,
            ARCHIVE_LIMIT,
            "restic_0.19.1_linux_amd64.bz2"
        ),
        Err(InstallError::AssetMissing(_))
    ));
    for status in [403_u16, 429, 500, 503] {
        let mut transport = RecordingTransport::default();
        transport.answers.insert(
            LATEST.to_string(),
            HttpAnswer {
                status,
                location: None,
                body: Vec::new(),
            },
        );
        assert!(
            matches!(
                fetch_release(&transport, &anchor),
                Err(InstallError::Network(_))
            ),
            "status {status}"
        );
    }
    for body in [
        "{\"draft\":false}",
        "{\"tag_name\":null}",
        "{\"tag_name\":123}",
        "not json",
        "[{\"tag_name\":\"v0.19.1\"}]",
        "{\"tag_name\":\"v0.19.1\",\"draft\":true}",
        "{\"tag_name\":\"v0.19.1\",\"prerelease\":true}",
    ] {
        let transport = RecordingTransport::default().ok(LATEST, body.as_bytes());
        assert!(
            matches!(
                fetch_release(&transport, &anchor),
                Err(InstallError::AssetMissing(_))
            ),
            "{body}"
        );
    }
}

/// Answers above their size limit stop the download.
#[test]
fn size_limits_stop_the_download() {
    let anchor = TrustAnchor::production();
    let size = |limit: u64| usize::try_from(limit + 1).expect("limit fits usize");
    let big = vec![b' '; size(RELEASE_INFO_LIMIT)];
    let transport = RecordingTransport::default().ok(LATEST, &big);
    assert!(matches!(
        fetch_release(&transport, &anchor),
        Err(InstallError::Network(_))
    ));
    let transport = RecordingTransport::default().ok(SUMS_0_19_1, &big[..size(SIGNATURE_LIMIT)]);
    assert!(matches!(
        fetch_asset(
            &transport,
            &anchor,
            SUMS_0_19_1,
            SIGNATURE_LIMIT,
            "SHA256SUMS.asc"
        ),
        Err(InstallError::Network(_))
    ));
}

/// Every error has the code of `dbus-api.md`.
#[test]
fn install_errors_map_to_their_codes() {
    use ResticInstallCode as C;
    let cases = [
        (InstallError::Network(String::new()), C::Network),
        (
            InstallError::HostNotAllowed(String::new()),
            C::HostNotAllowed,
        ),
        (InstallError::AssetMissing(String::new()), C::AssetMissing),
        (
            InstallError::UnsupportedArchitecture(String::new()),
            C::UnsupportedArchitecture,
        ),
        (InstallError::GpgvMissing, C::GpgvMissing),
        (
            InstallError::SignatureInvalid(String::new()),
            C::SignatureInvalid,
        ),
        (
            InstallError::ChecksumMismatch(String::new()),
            C::ChecksumMismatch,
        ),
        (
            InstallError::VersionMismatch {
                expected: String::new(),
                found: String::new(),
            },
            C::VersionMismatch,
        ),
        (
            InstallError::TooOld {
                found: String::new(),
                min: String::new(),
            },
            C::TooOld,
        ),
        (
            InstallError::OlderThanInstalled {
                release: String::new(),
                installed: String::new(),
            },
            C::TooOld,
        ),
        (InstallError::WriteFailed(String::new()), C::WriteFailed),
    ];
    for (error, code) in cases {
        assert_eq!(error.code(), Some(code), "{error:?}");
    }
    assert_eq!(InstallError::Cancelled.code(), None);
}

/// The test anchor replaces URL, key and fingerprint; plain HTTP only to 127.0.0.1.
#[test]
fn test_anchor_from_variables() {
    let key = tempfile::NamedTempFile::new().expect("key file");
    std::fs::write(key.path(), b"key bytes").expect("write key");
    let key_path = key.path().to_string_lossy().into_owned();
    let vars = |url: &'static str| {
        let key_path = key_path.clone();
        move |name: &str| match name {
            "KESTIC_TEST_RELEASE_URL" => Some(url.to_string()),
            "KESTIC_TEST_SIGNING_KEY" => Some(key_path.clone()),
            "KESTIC_TEST_SIGNING_FPR" => Some("ab cd".to_string()),
            _ => None,
        }
    };
    let anchor = TrustAnchor::from_lookup(vars("http://127.0.0.1:4711")).expect("test anchor");
    assert_eq!(
        anchor.release_api_url(),
        "http://127.0.0.1:4711/repos/restic/restic/releases/latest"
    );
    assert_eq!(
        anchor.asset_url(&Version::new(0, 19, 1), "SHA256SUMS"),
        "http://127.0.0.1:4711/restic/restic/releases/download/v0.19.1/SHA256SUMS"
    );
    assert_eq!(anchor.key(), b"key bytes");
    assert_eq!(anchor.fingerprint(), "ABCD");
    assert!(anchor.redirect_allowed(&Url::parse("http://127.0.0.1:4711/x").expect("url")));
    assert!(!anchor.redirect_allowed(&Url::parse("http://127.0.0.1:4712/x").expect("url")));
    assert!(matches!(
        TrustAnchor::from_lookup(vars("http://192.168.1.1:4711")),
        Err(InstallError::HostNotAllowed(_))
    ));
    assert_eq!(
        TrustAnchor::from_lookup(|_| None),
        Ok(TrustAnchor::production())
    );
}

/// A helper that leaves a process holding its output pipes still ends at the time limit.
#[test]
fn output_within_ends_while_a_background_process_holds_the_pipes() {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "sleep 4 & echo done"]);
    let started = Instant::now();
    let result = output_within(cmd, Duration::from_millis(300));
    assert!(
        matches!(&result, Err(err) if err.kind() == std::io::ErrorKind::TimedOut),
        "{result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}
