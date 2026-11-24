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

/// A data folder that is a symbolic link is refused; the permissions of its target stay.
#[test]
fn symlinked_data_dir_is_refused() {
    let root = tempfile::tempdir().expect("dir");
    let target = root.path().join("elsewhere");
    std::fs::create_dir(&target).expect("target");
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let link = root.path().join("kestic");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    // (a) refused
    assert!(matches!(
        prepare_data_dir(&link),
        Err(InstallError::WriteFailed(_))
    ));
    // (b) the target keeps its permissions
    let mode = std::fs::metadata(&target)
        .expect("target")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o755);
    // (c) a plain folder is narrowed to this user
    let plain = root.path().join("plain");
    std::fs::create_dir(&plain).expect("plain");
    std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    assert_eq!(prepare_data_dir(&plain), Ok(()));
    let mode = std::fs::metadata(&plain)
        .expect("plain")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
}

/// Leftovers of an interrupted installation are removed; nothing else in the folder is touched
/// and symbolic links are not followed.
#[test]
fn leftovers_of_an_interrupted_install_are_removed() {
    let root = tempfile::tempdir().expect("dir");
    let data = root.path().join("kestic");
    std::fs::create_dir(&data).expect("data dir");
    let outside = root.path().join("outside");
    std::fs::create_dir(&outside).expect("outside");
    std::fs::write(outside.join("keep"), b"x").expect("outside file");
    std::fs::write(data.join(".restic-download-0011aabb"), b"partial").expect("temp file");
    let gnupg = data.join(".restic-download-ccdd.gnupg");
    std::fs::create_dir(&gnupg).expect("gnupg");
    std::fs::write(gnupg.join("keyring.pgp"), b"key").expect("keyring");
    std::os::unix::fs::symlink(&outside, data.join(".restic-download-link")).expect("symlink");
    std::fs::write(data.join("restic"), b"binary").expect("binary");
    std::fs::write(data.join("known_hosts"), b"hosts").expect("known_hosts");

    // (a) three leftovers removed
    let cleanup = remove_leftovers(&data);
    assert_eq!(cleanup.removed, 3);
    assert!(cleanup.failed.is_empty(), "{:?}", cleanup.failed);
    let mut names: Vec<String> = std::fs::read_dir(&data)
        .expect("read")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    // (b) everything else stays
    assert_eq!(names, vec!["known_hosts".to_string(), "restic".to_string()]);
    // (c) the symbolic link's target is untouched
    assert!(outside.join("keep").is_file());
    // (d) a missing folder is no error
    let missing = remove_leftovers(&root.path().join("missing"));
    assert_eq!(missing.removed, 0);
    assert!(missing.failed.is_empty());
}

/// A leftover that cannot be removed does not keep the others.
#[test]
fn leftover_that_cannot_be_removed_does_not_stop_the_cleanup() {
    let root = tempfile::tempdir().expect("dir");
    let data = root.path().join("kestic");
    std::fs::create_dir(&data).expect("data dir");
    let stuck = data.join(".restic-download-aa.gnupg");
    std::fs::create_dir(&stuck).expect("stuck");
    std::fs::write(stuck.join("keyring.pgp"), b"key").expect("keyring");
    std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o500)).expect("chmod");
    std::fs::write(data.join(".restic-download-bb"), b"partial").expect("temp file");

    let cleanup = remove_leftovers(&data);
    std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o700)).expect("chmod back");
    // (a) the removable one is gone
    assert!(!data.join(".restic-download-bb").exists());
    assert_eq!(cleanup.removed, 1);
    // (b) the other is reported
    assert_eq!(cleanup.failed.len(), 1);
    assert_eq!(cleanup.failed[0].0, stuck);
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

/// Unpacking stops after [`UNPACKED_LIMIT`] bytes, so a small archive cannot fill the disk.
#[test]
fn unpack_stops_at_the_size_limit() {
    let mut zeros = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::best());
    let chunk = vec![0_u8; 1 << 20];
    for _ in 0..=(UNPACKED_LIMIT >> 20) {
        std::io::Write::write_all(&mut zeros, &chunk).expect("compress");
    }
    let archive = zeros.finish().expect("finish");
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("unpacked");
    let mut file = create_exclusive(&target).expect("temp file");

    let result = unpack(&archive, &mut file, "restic.bz2");
    assert!(
        matches!(&result, Err(InstallError::WriteFailed(detail)) if detail.contains("more than")),
        "{result:?}"
    );
    let size = std::fs::metadata(&target).expect("metadata").len();
    assert!(size <= UNPACKED_LIMIT + (64 << 10), "{size}");
}
