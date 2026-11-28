// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

//! M6-S18: a REST address with a login in it is refused where it is entered, and a config saved
//! with such an address still loads.

use kestic_core::config::parse_config_str;
use kestic_core::model::{Destination, DestinationId, DestinationKind, ModelError};
use kestic_i18n::messages::REST_ADDRESS_WITH_LOGIN;
use kestic_test_support::password::s3_key;
use std::path::Path;

fn rest(url: &str) -> Destination {
    Destination::new(
        "NAS",
        DestinationKind::Rest {
            url: url.to_string(),
            username: None,
        },
    )
    .expect("a REST destination passes the load-time rules")
}

/// The fixture config with a generated `user:password` in its REST address.
fn config_with_login() -> (String, String) {
    let login = format!("{}:{}", s3_key(12), s3_key(24));
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m6/config_rest_login.toml");
    let raw = std::fs::read_to_string(fixture).expect("read fixture");
    (raw.replace("@LOGIN@", &login), login)
}

/// AC-M6-18-1 (a), in `core`: an address with a login, or with a user name alone, fails the input
/// rules with the message that points to the fields «Username» and «Server password».
#[test]
fn ac_m6_18_1_login_in_rest_address_fails_the_input_rules() {
    let user = s3_key(12);
    let password = s3_key(24);
    for url in [
        format!("https://{user}:{password}@backup.example.org:8000/"),
        format!("http://{user}@127.0.0.1:8000/repo"),
    ] {
        let err = rest(&url).validate_input().expect_err("login refused");
        assert_eq!(err, ModelError::LoginInRestAddress, "(a) refusal");
        assert_eq!(err.to_string(), REST_ADDRESS_WITH_LOGIN, "(a) message");
    }
    assert!(
        REST_ADDRESS_WITH_LOGIN.contains("‘Username’")
            && REST_ADDRESS_WITH_LOGIN.contains("‘Server password’"),
        "(a) the message names both fields"
    );
}

/// AC-M6-18-3: (a) a REST address without a login passes the input rules, and (b) a config file
/// whose REST address carries a login still loads, with the address as it was saved.
#[test]
fn ac_m6_18_3_address_without_login_and_old_config_load() {
    for url in [
        "https://backup.example.org:8000/",
        "http://127.0.0.1:8000/repo",
        "https://nas.example.org/kestic/user@home",
    ] {
        assert_eq!(rest(url).validate_input(), Ok(()), "(a) accepted: {url}");
    }

    let (raw, login) = config_with_login();
    let config = parse_config_str(&raw).expect("(b) the config loads");
    let id = DestinationId::parse("rstlogin").expect("id");
    let dest = config
        .destination(&id)
        .expect("(b) the destination is there");
    assert_eq!(
        dest.kind,
        DestinationKind::Rest {
            url: format!("http://{login}@127.0.0.1:9/kestic"),
            username: None,
        },
        "(b) the address as saved"
    );
}
