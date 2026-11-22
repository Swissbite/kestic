// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Local repository folders: network addresses of the KDE file dialog (KIO, e.g. `smb://…`)
//! are no paths restic can use, and folders on a CIFS/SMB share get a warning.

use crate::model::{Destination, DestinationKind};
use crate::mount::{
    MountInspector, normalize_lexical_path, parse_mountinfo, resolve_mount_for_path,
};
use kestic_i18n::codes::ConnectionNoteCode;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Filesystem types of CIFS/SMB mounts in `/proc/self/mountinfo`.
const CIFS_FS_TYPES: [&str; 3] = ["cifs", "smb3", "smbfs"];

/// FUSE mounts of the desktop that show network shares as folders, with the first path
/// component below the mount point that marks an SMB share (`…/kio-fuse-XXXX/smb/nas/share`,
/// `…/gvfs/smb-share:server=nas,share=x`).
const FUSE_SMB_PREFIXES: [(&str, &str); 2] =
    [("fuse.kio-fuse", "smb"), ("fuse.gvfsd-fuse", "smb-share:")];

/// A folder chosen in the file dialog that is a network address, not a local path.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("not a local path: '{0}'")]
pub struct NotALocalPath(pub String);

/// URL scheme of `text` when it is a network address such as `smb://nas/share` or `sftp://host/x`
/// rather than a local path; `None` for plain paths and `file:` URLs.
pub fn kio_address_scheme(text: &str) -> Option<&str> {
    let (scheme, _) = text.split_once(':')?;
    let mut chars = scheme.chars();
    let starts_with_letter = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    let valid = starts_with_letter
        && scheme.len() >= 2
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    (valid && !scheme.eq_ignore_ascii_case("file")).then_some(scheme)
}

/// `address` without user name and password (`smb://user:pw@nas/x` → `smb://nas/x`), so it can
/// be shown in a message.
pub fn address_for_display(address: &str) -> String {
    let Some((scheme, rest)) = address.split_once("://") else {
        return address.to_string();
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    if let Some(at) = rest[..authority_end].rfind('@') {
        return format!("{scheme}://{}", &rest[at + 1..]);
    }
    // A password may contain `/`: `user:pa/ss@nas/x`.
    match rest.find('@') {
        Some(at) if rest[..authority_end].contains(':') => {
            format!("{scheme}://{}", &rest[at + 1..])
        }
        _ => address.to_string(),
    }
}

/// Local folder for the URL or path the folder dialog returns: a `file://` URL becomes its
/// decoded path, a plain path stays as it is, any other address is rejected.
pub fn local_path_from_dialog_url(url: &str) -> Result<PathBuf, NotALocalPath> {
    if let Some(rest) = strip_prefix_ignore_case(url, "file://") {
        let path_start = rest.find('/').unwrap_or(rest.len());
        let host = &rest[..path_start];
        if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
            return Err(NotALocalPath(address_for_display(url)));
        }
        let bytes = percent_decode(&rest[path_start..]);
        return Ok(PathBuf::from(OsString::from_vec(bytes)));
    }
    if kio_address_scheme(url).is_some() {
        return Err(NotALocalPath(address_for_display(url)));
    }
    Ok(PathBuf::from(url))
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// Decodes `%XX` escapes; a `%` without two hex digits stays as it is.
fn percent_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = text.get(i + 1..i + 3)
            && hex.bytes().all(|b| b.is_ascii_hexdigit())
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// Whether `path` lies on a CIFS/SMB share according to `inspector`'s mountinfo: a CIFS mount
/// or an SMB share shown by the FUSE mount of KIO or GVfs.
pub fn is_on_cifs_share(path: &Path, inspector: &dyn MountInspector) -> bool {
    let Ok(raw) = inspector.read_mountinfo() else {
        return false;
    };
    let entries = parse_mountinfo(&raw);
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let Some(entry) = resolve_mount_for_path(&resolved, &entries) else {
        return false;
    };
    if CIFS_FS_TYPES.contains(&entry.fs_type.as_str()) {
        return true;
    }
    let first_below_mount = normalize_lexical_path(&resolved)
        .strip_prefix(normalize_lexical_path(&entry.mount_point))
        .ok()
        .and_then(|rel| rel.components().next())
        .map(|c| c.as_os_str().to_string_lossy().into_owned());
    FUSE_SMB_PREFIXES.iter().any(|(fs_type, prefix)| {
        entry.fs_type == *fs_type
            && first_below_mount.as_deref().is_some_and(|first| {
                first == *prefix || (prefix.ends_with(':') && first.starts_with(prefix))
            })
    })
}

/// Notes the user confirms before `dest` is added, e.g. a folder on a CIFS/SMB share.
pub fn connection_notes(
    dest: &Destination,
    inspector: &dyn MountInspector,
) -> Vec<ConnectionNoteCode> {
    match &dest.kind {
        DestinationKind::Local { path } if is_on_cifs_share(path, inspector) => {
            vec![ConnectionNoteCode::CifsShare]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kio_scheme_is_found_only_for_network_addresses() {
        assert_eq!(kio_address_scheme("smb://nas/share"), Some("smb"));
        assert_eq!(kio_address_scheme("sftp://host/x"), Some("sftp"));
        assert_eq!(kio_address_scheme("remote:/"), Some("remote"));
        assert_eq!(kio_address_scheme("/mnt/backup"), None);
        assert_eq!(kio_address_scheme("/mnt/a:b"), None);
        assert_eq!(kio_address_scheme("file:///mnt/backup"), None);
        assert_eq!(kio_address_scheme("backups"), None);
    }

    #[test]
    fn display_drops_user_and_password() {
        assert_eq!(
            address_for_display("smb://user:secret@nas/share"),
            "smb://nas/share"
        );
        assert_eq!(address_for_display("smb://nas/share"), "smb://nas/share");
        assert_eq!(address_for_display("smb://nas/a@b"), "smb://nas/a@b");
        assert_eq!(address_for_display("smb://user:pa/ss@nas/x"), "smb://nas/x");
    }

    #[test]
    fn file_urls_are_decoded() {
        assert_eq!(
            local_path_from_dialog_url("file:///media/user/My%20Disk/b%C3%BCro"),
            Ok(PathBuf::from("/media/user/My Disk/büro"))
        );
        assert_eq!(
            local_path_from_dialog_url("file://localhost/mnt/x"),
            Ok(PathBuf::from("/mnt/x"))
        );
        assert_eq!(
            local_path_from_dialog_url("/mnt/100%"),
            Ok(PathBuf::from("/mnt/100%"))
        );
        assert_eq!(
            local_path_from_dialog_url("file:///a/%+1"),
            Ok(PathBuf::from("/a/%+1"))
        );
        assert_eq!(
            local_path_from_dialog_url("file://nas/share"),
            Err(NotALocalPath("file://nas/share".to_string()))
        );
    }
}
