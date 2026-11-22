// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Destination types the wizard offers and whether the programs they need are installed.
//!
//! The daemon checks its own `PATH`, because it is the one that starts restic and with it `ssh`
//! or `rclone`. Every call checks again, so a program installed in the meantime is found.

use kestic_i18n::codes::DestinationTypeCode;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::path::PathBuf;

/// A program a destination type needs on this computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequiredProgram {
    Ssh,
    Rclone,
}

impl RequiredProgram {
    /// Name of the executable searched in `PATH`.
    pub fn executable(self) -> &'static str {
        match self {
            Self::Ssh => "ssh",
            Self::Rclone => "rclone",
        }
    }

    /// Debian package that installs the program.
    pub fn package(self) -> &'static str {
        match self {
            Self::Ssh => "openssh-client",
            Self::Rclone => "rclone",
        }
    }

    /// Link that opens the package in Discover.
    pub fn appstream_url(self) -> String {
        format!("appstream://{}", self.package())
    }

    /// Terminal command that installs the package.
    pub fn install_command(self) -> String {
        format!("sudo apt install {}", self.package())
    }
}

/// Whether a destination type can be used now (`DestinationTypeDto`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationTypeStatus {
    pub kind: DestinationTypeCode,
    pub ready: bool,
    /// The program that is missing; `None` when the type is ready.
    pub missing_program: Option<RequiredProgram>,
    /// Package of the missing program; `None` when the type is ready.
    pub package: Option<String>,
    /// Discover link for the missing program; `None` when the type is ready.
    pub appstream_url: Option<String>,
}

/// The program `kind` needs locally, if any.
pub fn required_program(kind: DestinationTypeCode) -> Option<RequiredProgram> {
    match kind {
        DestinationTypeCode::Sftp => Some(RequiredProgram::Ssh),
        DestinationTypeCode::Rclone => Some(RequiredProgram::Rclone),
        DestinationTypeCode::Drive
        | DestinationTypeCode::Folder
        | DestinationTypeCode::Rest
        | DestinationTypeCode::S3
        | DestinationTypeCode::B2
        | DestinationTypeCode::Azure
        | DestinationTypeCode::Gs
        | DestinationTypeCode::Swift => None,
    }
}

/// True when `search_path` (a `PATH` value) holds a file called `name` that this user may
/// execute. Only absolute entries count: restic starts `ssh` and `rclone` the way Go does, and
/// Go ignores programs found through a relative entry (or an empty one, the current directory).
pub fn program_in_path(name: &str, search_path: Option<&OsStr>) -> bool {
    let Some(search_path) = search_path else {
        return false;
    };
    let absolute: Vec<PathBuf> = std::env::split_paths(search_path)
        .filter(|dir| dir.is_absolute())
        .collect();
    let Ok(absolute) = std::env::join_paths(absolute) else {
        return false;
    };
    which::which_in(name, Some(absolute), "/").is_ok()
}

/// All destination types in the order of the wizard's cards, each checked against
/// `search_path` (the `PATH` of the daemon).
pub fn destination_types(search_path: Option<&OsStr>) -> Vec<DestinationTypeStatus> {
    DestinationTypeCode::ALL
        .into_iter()
        .map(|kind| {
            let missing = required_program(kind)
                .filter(|program| !program_in_path(program.executable(), search_path));
            DestinationTypeStatus {
                kind,
                ready: missing.is_none(),
                missing_program: missing,
                package: missing.map(|p| p.package().to_string()),
                appstream_url: missing.map(RequiredProgram::appstream_url),
            }
        })
        .collect()
}

/// A card of the wizard's step «destination type». «Cloud Storage» groups the cloud kinds that
/// need no local program; the others show one kind each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationTypeCard {
    Drive,
    Folder,
    Sftp,
    Rest,
    Cloud,
    Rclone,
}

impl DestinationTypeCard {
    /// The card that shows `kind`.
    pub fn of(kind: DestinationTypeCode) -> Self {
        match kind {
            DestinationTypeCode::Drive => Self::Drive,
            DestinationTypeCode::Folder => Self::Folder,
            DestinationTypeCode::Sftp => Self::Sftp,
            DestinationTypeCode::Rest => Self::Rest,
            DestinationTypeCode::S3
            | DestinationTypeCode::B2
            | DestinationTypeCode::Azure
            | DestinationTypeCode::Gs
            | DestinationTypeCode::Swift => Self::Cloud,
            DestinationTypeCode::Rclone => Self::Rclone,
        }
    }
}

/// One card with its readiness and, when it needs an installation, what to install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationTypeCardStatus {
    pub card: DestinationTypeCard,
    pub kinds: Vec<DestinationTypeCode>,
    /// All kinds of the card are ready.
    pub ready: bool,
    pub missing_program: Option<RequiredProgram>,
    pub package: Option<String>,
    pub appstream_url: Option<String>,
    pub install_command: Option<String>,
}

/// Groups `types` into cards and puts the ready cards before those that need an installation;
/// within each group the cards keep their order.
pub fn cards_ready_first(types: &[DestinationTypeStatus]) -> Vec<DestinationTypeCardStatus> {
    let mut cards: Vec<DestinationTypeCardStatus> = Vec::new();
    for status in types {
        let card = DestinationTypeCard::of(status.kind);
        let index = match cards.iter().position(|c| c.card == card) {
            Some(index) => index,
            None => {
                cards.push(DestinationTypeCardStatus {
                    card,
                    kinds: Vec::new(),
                    ready: true,
                    missing_program: None,
                    package: None,
                    appstream_url: None,
                    install_command: None,
                });
                cards.len() - 1
            }
        };
        let entry = &mut cards[index];
        entry.kinds.push(status.kind);
        entry.ready &= status.ready;
        if entry.missing_program.is_none()
            && let Some(program) = status.missing_program
        {
            entry.missing_program = Some(program);
            entry.package.clone_from(&status.package);
            entry.appstream_url.clone_from(&status.appstream_url);
            entry.install_command = Some(program.install_command());
        }
    }
    let (mut ordered, needs_installation): (Vec<_>, Vec<_>) =
        cards.into_iter().partition(|c| c.ready);
    ordered.extend(needs_installation);
    ordered
}
