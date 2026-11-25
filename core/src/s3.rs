// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! S3-compatible destinations: endpoint, bucket and path checks and the restic repository
//! address `s3:<endpoint>/<bucket>[/<path>]`.

use crate::model::ModelError;

/// Checks the fields of an S3 destination: an `http(s)://` endpoint with a host and without a
/// path, a valid bucket name and a path without `..` segments.
pub fn validate(endpoint: &str, bucket: &str, path: Option<&str>) -> Result<(), ModelError> {
    normalized_endpoint(endpoint)?;
    if !is_valid_bucket_name(bucket) {
        return Err(ModelError::InvalidDestination(format!(
            "invalid S3 bucket name '{bucket}': 3 to 63 lowercase letters, digits, dots or \
             hyphens, starting and ending with a letter or digit"
        )));
    }
    path.map_or(Ok(()), check_path)
}

/// Checks the path inside the bucket: no empty, `.` or `..` segments and no character that would
/// change the address restic reads.
pub fn check_path(path: &str) -> Result<(), ModelError> {
    let trimmed = trimmed_path(path);
    if !trimmed.is_empty() && trimmed.split('/').any(|seg| matches!(seg, "" | "." | "..")) {
        return Err(ModelError::InvalidDestination(format!(
            "S3 path must not contain empty, '.' or '..' segments: '{path}'"
        )));
    }
    // restic reads the address as URL: `#` and `?` would cut the path, `%` would decode it.
    // Spaces and quotes would split the address in restic's messages, which Kestic reads.
    if path
        .chars()
        .any(|c| matches!(c, '#' | '?' | '%' | '"') || c.is_control() || c.is_whitespace())
    {
        return Err(ModelError::InvalidDestination(format!(
            "S3 path must not contain '#', '?', '%', quotes, spaces or control characters: '{path}'"
        )));
    }
    Ok(())
}

/// Checks the optional region: it goes to restic as `AWS_DEFAULT_REGION`, so it holds no spaces
/// or control characters (a NUL byte would stop every restic start).
pub fn validate_region(region: Option<&str>) -> Result<(), ModelError> {
    match region.map(str::trim) {
        Some(r) if r.chars().any(|c| c.is_control() || c.is_whitespace()) => {
            Err(ModelError::InvalidDestination(format!(
                "S3 region must not contain spaces or control characters: '{}'",
                r.escape_default()
            )))
        }
        _ => Ok(()),
    }
}

/// Whether `name` follows the S3 bucket naming rules: 3–63 characters of lowercase letters,
/// digits, dots and hyphens, starting and ending with a letter or digit, no two dots in a row and
/// not shaped like an IPv4 address.
pub fn is_valid_bucket_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if !(3..=63).contains(&bytes.len()) {
        return false;
    }
    let allowed = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-';
    let alnum = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    bytes.iter().all(|&c| allowed(c))
        && alnum(bytes[0])
        && alnum(bytes[bytes.len() - 1])
        && !name.contains("..")
        && name.parse::<std::net::Ipv4Addr>().is_err()
}

/// The endpoint as ASCII URL without a trailing slash (`https://s3.example.com:9000`);
/// internationalized host names become Punycode.
pub fn normalized_endpoint(endpoint: &str) -> Result<String, ModelError> {
    let trimmed = endpoint.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        return Err(ModelError::InvalidDestination(
            "S3 endpoint must start with http:// or https://".to_string(),
        ));
    }
    let parsed = url::Url::parse(trimmed)
        .map_err(|err| ModelError::InvalidDestination(format!("invalid S3 endpoint: {err}")))?;
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err(ModelError::InvalidDestination(
            "S3 endpoint must include a host".to_string(),
        ));
    }
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(ModelError::InvalidDestination(
            "S3 endpoint must not contain a path; enter bucket and path separately".to_string(),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ModelError::InvalidDestination(
            "S3 endpoint must not contain access data; enter the keys separately".to_string(),
        ));
    }
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

/// restic repository address `s3:<endpoint>/<bucket>[/<path>]`.
pub fn repository_uri(endpoint: &str, bucket: &str, path: Option<&str>) -> String {
    let endpoint = normalized_endpoint(endpoint)
        .unwrap_or_else(|_| endpoint.trim().trim_end_matches('/').to_string());
    format!("s3:{}", location(&endpoint, bucket, path))
}

/// Comparison key of the repository location: endpoint with lowercase host, bucket and path
/// without surrounding slashes.
pub fn location_key(endpoint: &str, bucket: &str, path: Option<&str>) -> String {
    let endpoint = normalized_endpoint(endpoint)
        .unwrap_or_else(|_| endpoint.trim().trim_end_matches('/').to_ascii_lowercase());
    format!("s3:{}", location(&endpoint, bucket, path))
}

/// `<endpoint>/<bucket>[/<path>]` as shown to the user.
pub fn display_location(endpoint: &str, bucket: &str, path: Option<&str>) -> String {
    location(endpoint.trim().trim_end_matches('/'), bucket, path)
}

fn location(endpoint: &str, bucket: &str, path: Option<&str>) -> String {
    let bucket = bucket.trim();
    match path.map(trimmed_path).filter(|p| !p.is_empty()) {
        Some(p) => format!("{endpoint}/{bucket}/{p}"),
        None => format!("{endpoint}/{bucket}"),
    }
}

fn trimmed_path(path: &str) -> &str {
    path.trim().trim_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_names_follow_the_s3_rules() {
        for ok in ["backup", "my-bucket.2026", "abc", &"a".repeat(63)] {
            assert!(is_valid_bucket_name(ok), "{ok} rejected");
        }
        for bad in [
            "ab",
            &"a".repeat(64),
            "Backup",
            "-backup",
            "backup-",
            "my..bucket",
            "my_bucket",
            "192.168.1.10",
            "",
        ] {
            assert!(!is_valid_bucket_name(bad), "{bad} accepted");
        }
    }

    #[test]
    fn repository_uri_joins_endpoint_bucket_and_path() {
        assert_eq!(
            repository_uri("https://s3.example.com:9000/", "backup", None),
            "s3:https://s3.example.com:9000/backup"
        );
        assert_eq!(
            repository_uri("http://127.0.0.1:9000", "backup", Some("/laptop/home/")),
            "s3:http://127.0.0.1:9000/backup/laptop/home"
        );
        assert_eq!(
            repository_uri("https://s3.example.com", "backup", Some("  ")),
            "s3:https://s3.example.com/backup"
        );
    }

    #[test]
    fn endpoint_needs_scheme_and_host_without_path() {
        assert!(normalized_endpoint("s3.example.com").is_err());
        assert!(normalized_endpoint("ftp://s3.example.com").is_err());
        assert!(normalized_endpoint("https://s3.example.com/backup").is_err());
        let with_login = format!(
            "https://{}:{}@s3.example.com",
            kestic_test_support::password::s3_key(16),
            kestic_test_support::password::s3_key(32)
        );
        assert!(normalized_endpoint(&with_login).is_err());
        assert_eq!(
            normalized_endpoint("HTTPS://S3.Example.com:9000/").expect("valid"),
            "https://s3.example.com:9000"
        );
    }

    #[test]
    fn path_rejects_parent_segments() {
        assert!(validate("https://s3.example.com", "backup", Some("a/../b")).is_err());
        assert!(validate("https://s3.example.com", "backup", Some("a/b")).is_ok());
    }

    #[test]
    fn region_rejects_spaces_and_control_characters() {
        for bad in ["eu west 1", "x\u{0}';id;'", "eu\tcentral"] {
            assert!(validate_region(Some(bad)).is_err(), "{bad:?} accepted");
        }
        assert!(validate_region(Some(" eu-central-1 ")).is_ok());
        assert!(validate_region(None).is_ok());
    }

    #[test]
    fn path_rejects_characters_that_change_the_url() {
        for bad in [
            "laptop#one",
            "a?b",
            "a%41",
            "a/%2e%2e/b",
            "100%",
            "a\nb",
            "a//b",
            "a/./b",
            ".",
            "my forbidden zone",
            "my\"p",
            "tab\there",
        ] {
            assert!(
                validate("https://s3.example.com", "backup", Some(bad)).is_err(),
                "{bad} accepted"
            );
        }
    }
}
