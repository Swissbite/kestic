// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::os::unix::fs::PermissionsExt;

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write stand-in");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    path
}

/// AC-M5-06-9: a new binary must report the release's version, at least 0.18.0, in
/// `version --json`; anything else refuses it.
#[test]
fn ac_m5_06_9_unusable_payload_and_version_forms() {
    let dir = tempfile::tempdir().expect("dir");
    let release = Version::new(0, 19, 1);
    let limit = Duration::from_secs(10);
    let json = |v: &str| {
        format!("#!/bin/sh\necho '{{\"message_type\":\"version\",\"version\":\"{v}\"}}'\n")
    };

    let good = script(dir.path(), "good", &json("0.19.1"));
    assert_eq!(
        check_binary_version(&good, &release, limit),
        Ok(release.clone())
    );

    let dev = script(dir.path(), "dev", &json("0.19.1-dev"));
    assert!(matches!(
        check_binary_version(&dev, &release, limit),
        Err(InstallError::VersionMismatch { .. })
    ));
    let old = script(dir.path(), "old", &json("0.17.99"));
    assert!(matches!(
        check_binary_version(&old, &Version::new(0, 17, 99), limit),
        Err(InstallError::TooOld { .. })
    ));
    let not_json = script(dir.path(), "text", "#!/bin/sh\necho 'restic 0.19.1'\n");
    assert!(matches!(
        check_binary_version(&not_json, &release, limit),
        Err(InstallError::VersionMismatch { .. })
    ));
    let failing = script(
        dir.path(),
        "failing",
        &format!("{}exit 1\n", json("0.19.1")),
    );
    assert!(matches!(
        check_binary_version(&failing, &release, limit),
        Err(InstallError::VersionMismatch { .. })
    ));
    let not_executable = script(dir.path(), "data", "no shebang, no ELF\n");
    assert!(matches!(
        check_binary_version(&not_executable, &release, limit),
        Err(InstallError::VersionMismatch { .. })
    ));

    let hanging = script(dir.path(), "hanging", "#!/bin/sh\nexec sleep 30\n");
    let started = std::time::Instant::now();
    assert!(matches!(
        check_binary_version(&hanging, &release, Duration::from_millis(300)),
        Err(InstallError::VersionMismatch { .. })
    ));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );

    // archives that do not unpack to a program
    for archive in [Vec::new(), b"not bzip2".to_vec()] {
        let target = dir.path().join("unpacked");
        let mut file = create_exclusive(&target).expect("temp file");
        assert!(matches!(
            unpack(&archive, &mut file, "restic.bz2"),
            Err(InstallError::AssetMissing(_))
        ));
        std::fs::remove_file(&target).expect("remove");
    }
}

/// AC-M5-06-7: the temporary file is new, private and never follows what is already there.
#[test]
fn ac_m5_06_7_exclusive_temp_file() {
    let dir = tempfile::tempdir().expect("dir");
    let victim = dir.path().join("victim");
    std::fs::write(&victim, b"keep").expect("victim");
    let planted = dir.path().join(format!("{TEMP_PREFIX}planted"));
    std::os::unix::fs::symlink(&victim, &planted).expect("symlink");
    assert_eq!(
        create_exclusive(&planted).map(|_| ()).map_err(|e| e.kind()),
        Err(std::io::ErrorKind::AlreadyExists)
    );
    assert_eq!(std::fs::read(&victim).expect("victim"), b"keep");

    let fresh = dir.path().join(format!("{TEMP_PREFIX}fresh"));
    create_exclusive(&fresh).expect("new file");
    let mode = std::fs::metadata(&fresh)
        .expect("meta")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);

    let a = temp_name();
    let b = temp_name();
    assert!(a.starts_with(TEMP_PREFIX), "{a}");
    assert!(b.starts_with(TEMP_PREFIX), "{b}");
    assert_ne!(a, b);
}

/// The data folder follows the XDG base directory rules.
#[test]
fn data_dir_from_xdg_data_home() {
    let home = OsStr::new("/home/u");
    assert_eq!(
        kestic_data_dir(Some(home), Some(OsStr::new("/data"))),
        Some(PathBuf::from("/data/kestic"))
    );
    assert_eq!(
        kestic_data_dir(Some(home), Some(OsStr::new("relative"))),
        Some(PathBuf::from("/home/u/.local/share/kestic"))
    );
    assert_eq!(
        kestic_data_dir(Some(home), Some(OsStr::new(""))),
        Some(PathBuf::from("/home/u/.local/share/kestic"))
    );
    assert_eq!(
        kestic_data_dir(Some(home), None),
        Some(PathBuf::from("/home/u/.local/share/kestic"))
    );
    assert_eq!(kestic_data_dir(None, None), None);
    assert_eq!(kestic_data_dir(Some(OsStr::new("relative")), None), None);
}
