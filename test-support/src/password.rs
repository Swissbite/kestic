// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Generated test secrets: strong passwords full of characters that break careless quoting,
//! alphanumeric backend keys, and a leak assertion that never prints the secret it looks for.

use crate::random::random_index;

/// Characters every [`strong_password`] contains: shell metacharacters, quotes, escapes, a
/// space, non-ASCII letters, a symbol outside the BMP and a combining sequence.
const REQUIRED_PIECES: &[&str] = &[
    "$", "\"", "'", "`", "\\", "!", "*", ";", "&", "|", "#", "%", "=", "(", ")", "<", ">", "~",
    ",", ":", "@", "/", "{", "}", "[", "]", "?", "^", " ", "ä", "ß", "€", "🙂", "e\u{301}",
];

/// Characters a password may start with: an option-like dash or a comment-like hash.
const FIRST_CHARS: &[&str] = &["-", "#"];

/// Length of the alphanumeric marker inside every [`strong_password`].
const MARKER_LEN: usize = 12;

/// Separator around the marker, so neighbouring pieces never extend the marker run.
const MARKER_FENCE: &str = "_";

const ALPHANUMERIC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// `len` random ASCII letters and digits.
fn random_alphanumeric(len: usize) -> String {
    (0..len)
        .map(|_| char::from(ALPHANUMERIC[random_index(ALPHANUMERIC.len())]))
        .collect()
}

/// A random password of at least 32 characters for repositories and backends in tests.
///
/// It starts with `-` or `#`, contains every piece of [`REQUIRED_PIECES`] in random order and a
/// fenced alphanumeric marker of 12 characters (which [`assert_no_secret`] also looks for), and
/// ends with a letter or digit. It never contains control characters or line breaks, and never
/// starts or ends with whitespace: restic strips that from password files and password commands.
pub fn strong_password() -> String {
    let mut middle: Vec<String> = REQUIRED_PIECES.iter().map(|p| (*p).to_string()).collect();
    middle.push(format!(
        "{MARKER_FENCE}{}{MARKER_FENCE}",
        random_alphanumeric(MARKER_LEN)
    ));
    for i in (1..middle.len()).rev() {
        middle.swap(i, random_index(i + 1));
    }
    let first = FIRST_CHARS[random_index(FIRST_CHARS.len())];
    format!("{first}{}{}", middle.concat(), random_alphanumeric(1))
}

/// A random key of `len` ASCII letters and digits, for backends whose credential syntax
/// reserves separators (e.g. `rclone serve s3 --auth-key <key>,<secret>`).
pub fn s3_key(len: usize) -> String {
    random_alphanumeric(len)
}

/// The fenced marker of a [`strong_password`]: it survives any quoting or escaping, so finding
/// it in a text reveals a leaked password in whatever form it was written.
fn marker(secret: &str) -> &str {
    secret
        .split(MARKER_FENCE)
        .find(|run| run.len() == MARKER_LEN && run.bytes().all(|b| b.is_ascii_alphanumeric()))
        .expect("secret has no marker; generate it with strong_password()")
}

/// The secret as a JSON or TOML basic string writes it, without the surrounding quotes.
fn escaped(secret: &str) -> String {
    let quoted = serde_json::to_string(secret).expect("serialize secret");
    quoted[1..quoted.len() - 1].to_string()
}

/// Asserts that `haystack` contains `secret` in no form: raw, JSON/TOML-escaped, or (through its
/// marker) quoted for a shell. The failure message names only the form, never the secret.
pub fn assert_no_secret(haystack: &str, secret: &str, what: &str) {
    let forms = [
        ("raw", secret.to_string()),
        ("escaped", escaped(secret)),
        ("marker", marker(secret).to_string()),
    ];
    for (form, needle) in forms {
        assert!(
            !haystack.contains(&needle),
            "{what} contains the secret ({form} form)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strong_password_contains_every_required_piece() {
        let pw = strong_password();
        // (a) every required piece is present
        for piece in REQUIRED_PIECES {
            assert!(pw.contains(piece), "required piece {piece:?} missing");
        }
        // (b) it starts with an option-like or comment-like character
        assert!(FIRST_CHARS.iter().any(|c| pw.starts_with(c)));
        // (c) it ends with a letter or digit
        assert!(pw.chars().last().is_some_and(|c| c.is_ascii_alphanumeric()));
        // (d) it is at least 32 characters long
        assert!(pw.chars().count() >= 32);
    }

    #[test]
    fn strong_password_has_clean_edges_and_no_control_characters() {
        let pw = strong_password();
        // (a) no whitespace at either edge
        assert_eq!(pw.trim(), pw);
        // (b) no control characters anywhere
        assert!(!pw.chars().any(char::is_control));
    }

    #[test]
    fn strong_passwords_differ() {
        assert_ne!(strong_password(), strong_password());
    }

    #[test]
    fn s3_key_is_alphanumeric() {
        let key = s3_key(20);
        assert_eq!(key.len(), 20);
        assert!(key.bytes().all(|b| b.is_ascii_alphanumeric()));
    }

    #[test]
    #[should_panic(expected = "(raw form)")]
    fn assert_no_secret_finds_raw_form() {
        let pw = strong_password();
        assert_no_secret(&format!("x{pw}x"), &pw, "text");
    }

    #[test]
    #[should_panic(expected = "(escaped form)")]
    fn assert_no_secret_finds_escaped_form() {
        let pw = strong_password();
        let json = serde_json::to_string(&serde_json::json!({ "pw": pw })).expect("json");
        assert_no_secret(&json, &pw, "json");
    }

    #[test]
    #[should_panic(expected = "(marker form)")]
    fn assert_no_secret_finds_marker_in_shell_quoting() {
        let pw = strong_password();
        let shell = format!("'{}'", pw.replace('\'', "'\\''"));
        assert_no_secret(&shell, &pw, "shell");
    }

    #[test]
    fn assert_no_secret_accepts_text_without_secret() {
        let pw = strong_password();
        assert_no_secret("nothing to see here", &pw, "text");
    }
}
