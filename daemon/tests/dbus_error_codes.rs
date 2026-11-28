// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! D-Bus errors are typed codes (`docs/spec/dbus-api.md` §Fehler-Namen): every
//! `net.daester.Kestic.Error.*` variant survives the trip over D-Bus as the same variant, its
//! error name is its code, and together the variants cover every `ServiceErrorCode`.

use kestic_daemon::dbus_api::KesticDbusError;
use kestic_i18n::codes::{BlockedReason, ServiceErrorCode};
use std::collections::HashSet;
use zbus::DBusError;

/// What a client receives for `err`: a `MethodError` with the error name and the message.
fn over_the_wire(err: &KesticDbusError) -> zbus::Error {
    let call = zbus::message::Message::method_call("/net/daester/Kestic", "Probe")
        .build(&())
        .expect("method call message");
    let reply = zbus::message::Message::error(&call.header(), err.name())
        .build(&(err.description().unwrap_or_default(),))
        .expect("error message");
    zbus::Error::MethodError(
        err.name().to_owned().into(),
        err.description().map(str::to_string),
        reply,
    )
}

fn every_named_variant() -> Vec<KesticDbusError> {
    let d = || "raw detail".to_string();
    vec![
        KesticDbusError::InvalidInput(d()),
        KesticDbusError::NotFound(d()),
        KesticDbusError::InUse(d()),
        KesticDbusError::ManualRunBlocked(d()),
        KesticDbusError::DestinationUnreachable(d()),
        KesticDbusError::WrongPassword(d()),
        KesticDbusError::AuthFailed(d()),
        KesticDbusError::SecretStoreLocked(d()),
        KesticDbusError::ResticUnavailable(d()),
        KesticDbusError::AlreadyRunning(d()),
        KesticDbusError::LockInUse(d()),
        KesticDbusError::DuplicateLocation(d()),
        KesticDbusError::HostKeyUnknown(d()),
        KesticDbusError::HostKeyChanged(d()),
        KesticDbusError::PreviewRequired(d()),
        KesticDbusError::RestoreRunning(d()),
        KesticDbusError::Conflict(d()),
        KesticDbusError::Internal(d()),
    ]
}

#[test]
fn every_dbus_error_round_trips_as_its_code() {
    let mut codes = HashSet::new();
    for err in every_named_variant() {
        let code = err.code();
        assert_eq!(
            err.name().as_str(),
            format!("net.daester.Kestic.Error.{code:?}"),
            "error name is the code"
        );
        let back = KesticDbusError::from(over_the_wire(&err));
        assert_eq!(back.code(), code, "{code:?} survives the D-Bus round trip");
        assert_eq!(
            back.description(),
            Some("raw detail"),
            "{code:?} keeps its raw detail"
        );
        codes.insert(code);
    }
    let transport = KesticDbusError::from(zbus::Error::Failure("no bus".to_string()));
    assert_eq!(transport.code(), ServiceErrorCode::DaemonUnreachable);
    codes.insert(transport.code());
    let all: HashSet<ServiceErrorCode> = ServiceErrorCode::ALL.into_iter().collect();
    assert_eq!(codes, all, "the variants cover every ServiceErrorCode");
}

#[test]
fn manual_run_blocked_carries_the_reason_code() {
    let err =
        KesticDbusError::ManualRunBlocked(BlockedReason::ConditionNotMet.as_str().to_string());
    let back = KesticDbusError::from(over_the_wire(&err));
    assert_eq!(back.blocked_reason(), Some(BlockedReason::ConditionNotMet));
    assert_eq!(
        KesticDbusError::Internal("x".to_string()).blocked_reason(),
        None
    );
}
