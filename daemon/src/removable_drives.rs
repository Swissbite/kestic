// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Mounted removable drives from UDisks2 (`ListRemovableDrives`), offered as destinations.

use crate::dbus_api::RemovableDriveDto;
use crate::monitors::{MonitorBusConfig, connect_target_bus, name_has_owner};
use std::collections::HashMap;
use zbus::{ObjectPath, OwnedObjectPath, OwnedVariant, Proxy, block_on};

pub const UDISKS2_DEST: &str = "org.freedesktop.UDisks2";
const UDISKS2_PATH: &str = "/org/freedesktop/UDisks2";
const BLOCK_IFACE: &str = "org.freedesktop.UDisks2.Block";
const FILESYSTEM_IFACE: &str = "org.freedesktop.UDisks2.Filesystem";
const DRIVE_IFACE: &str = "org.freedesktop.UDisks2.Drive";

/// Properties by interface name of one UDisks2 object.
pub type ObjectInterfaces = HashMap<String, HashMap<String, OwnedVariant>>;

/// Result of `org.freedesktop.DBus.ObjectManager.GetManagedObjects`.
pub type ManagedObjects = HashMap<OwnedObjectPath, ObjectInterfaces>;

fn prop<'a>(object: &'a ObjectInterfaces, iface: &str, name: &str) -> Option<&'a OwnedVariant> {
    object.get(iface)?.get(name)
}

fn prop_bool(object: &ObjectInterfaces, iface: &str, name: &str) -> bool {
    prop(object, iface, name).is_some_and(|v| bool::try_from(v).unwrap_or(false))
}

fn prop_str(object: &ObjectInterfaces, iface: &str, name: &str) -> String {
    prop(object, iface, name)
        .and_then(|v| <&str>::try_from(v).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn prop_path(object: &ObjectInterfaces, iface: &str, name: &str) -> Option<String> {
    let value = prop(object, iface, name)?;
    let path = <&ObjectPath<'_>>::try_from(value).ok()?.as_str();
    (path != "/").then(|| path.to_string())
}

/// Bytes of an `ay` property without the trailing NUL UDisks2 adds.
fn bytes_to_string(mut bytes: Vec<u8>) -> String {
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn first_mount_point(object: &ObjectInterfaces) -> Option<String> {
    let value = prop(object, FILESYSTEM_IFACE, "MountPoints")?.clone();
    Vec::<Vec<u8>>::try_from(value)
        .ok()?
        .into_iter()
        .map(bytes_to_string)
        .find(|mp| !mp.is_empty())
}

/// The drive object a block device belongs to; an unlocked encrypted device takes the drive of
/// the device it is backed by.
fn drive_of<'a>(
    objects: &'a ManagedObjects,
    block: &ObjectInterfaces,
) -> Option<&'a ObjectInterfaces> {
    let drive_path = prop_path(block, BLOCK_IFACE, "Drive").or_else(|| {
        let backing = prop_path(block, BLOCK_IFACE, "CryptoBackingDevice")?;
        let backing_obj = objects.get(&OwnedObjectPath::try_from(backing).ok()?)?;
        prop_path(backing_obj, BLOCK_IFACE, "Drive")
    })?;
    objects.get(&OwnedObjectPath::try_from(drive_path).ok()?)
}

/// Name shown for a drive: the filesystem label, else vendor and model, else the device.
fn drive_name(
    path: &OwnedObjectPath,
    block: &ObjectInterfaces,
    drive: &ObjectInterfaces,
) -> String {
    let label = prop_str(block, BLOCK_IFACE, "IdLabel");
    if !label.is_empty() {
        return label;
    }
    let vendor_model = [
        prop_str(drive, DRIVE_IFACE, "Vendor"),
        prop_str(drive, DRIVE_IFACE, "Model"),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    if !vendor_model.is_empty() {
        return vendor_model;
    }
    path.as_str()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Mounted filesystems on drives UDisks2 marks as removable (USB disks, SD cards, …), sorted
/// by name and mount point.
pub fn removable_drives(objects: &ManagedObjects) -> Vec<RemovableDriveDto> {
    let mut drives: Vec<RemovableDriveDto> = objects
        .iter()
        .filter(|(_, object)| !prop_bool(object, BLOCK_IFACE, "HintIgnore"))
        .filter_map(|(path, object)| {
            let mount_point = first_mount_point(object)?;
            let drive = drive_of(objects, object)?;
            prop_bool(drive, DRIVE_IFACE, "Removable").then(|| RemovableDriveDto {
                name: drive_name(path, object, drive),
                mount_point,
            })
        })
        .collect();
    drives.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.mount_point.cmp(&b.mount_point))
    });
    drives
}

/// Asks UDisks2 for its objects; empty when UDisks2 is not running or the call fails.
pub fn list_removable_drives(cfg: &MonitorBusConfig) -> Vec<RemovableDriveDto> {
    block_on(async move {
        let Ok(conn) = connect_target_bus(cfg).await else {
            return Vec::new();
        };
        if !name_has_owner(&conn, UDISKS2_DEST).await {
            return Vec::new();
        }
        let proxy = match Proxy::new(
            &conn,
            UDISKS2_DEST,
            UDISKS2_PATH,
            "org.freedesktop.DBus.ObjectManager",
        )
        .await
        {
            Ok(proxy) => proxy,
            Err(err) => {
                eprintln!("kestic-daemon: UDisks2 ObjectManager proxy failed: {err}");
                return Vec::new();
            }
        };
        match proxy
            .call::<_, _, ManagedObjects>("GetManagedObjects", &())
            .await
        {
            Ok(objects) => removable_drives(&objects),
            Err(err) => {
                eprintln!("kestic-daemon: UDisks2 GetManagedObjects failed: {err}");
                Vec::new()
            }
        }
    })
}
