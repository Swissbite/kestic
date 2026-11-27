// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

//! The `<releases>` entry of the AppStream metainfo in the `.deb`. Like the package version it
//! comes from the Git tags (or `KESTIC_VERSION`), never from a file in a commit: the template
//! `packaging/net.daester.Kestic.metainfo.xml` has no release, and `just deb` adds the one of the
//! build version, dated with the commit the build comes from.

use crate::commit::{git, is_own_checkout};
use std::path::Path;

/// Environment variable of reproducible builds: seconds since 1970 that stand for the build date.
pub const SOURCE_DATE_VAR: &str = "SOURCE_DATE_EPOCH";

/// `metainfo` with a `<releases>` block for `version` on `date` (`YYYY-MM-DD`) added as the last
/// child of `<component>`. Fails for a template that already names a release, so none is ever
/// written by hand.
pub fn with_release(metainfo: &str, version: &str, date: &str) -> Result<String, String> {
    if metainfo.contains("<release") {
        return Err(
            "the metainfo template must not contain a <release> entry: it comes from the Git tags"
                .to_owned(),
        );
    }
    if !is_iso_date(date) {
        return Err(format!("release date {date:?} is no YYYY-MM-DD date"));
    }
    if version.is_empty()
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'~'))
    {
        return Err(format!("release version {version:?} is no Debian version"));
    }
    let end = metainfo
        .rfind("</component>")
        .ok_or("the metainfo has no </component>")?;
    let releases = format!(
        "  <releases>\n    <release version=\"{version}\" date=\"{date}\"/>\n  </releases>\n"
    );
    Ok(format!(
        "{}{releases}{}",
        &metainfo[..end],
        &metainfo[end..]
    ))
}

/// UTC date of the release entry: `SOURCE_DATE_EPOCH` when set and not empty, else the committer
/// time of `HEAD` in the checkout at `workspace_root`.
pub fn release_date(
    workspace_root: &Path,
    source_date_epoch: Option<&str>,
) -> Result<String, String> {
    if let Some(value) = source_date_epoch.filter(|v| !v.is_empty()) {
        return epoch_seconds(value)
            .map(date_of_epoch)
            .ok_or_else(|| format!("{SOURCE_DATE_VAR}={value:?} is no number of seconds"));
    }
    if !is_own_checkout(workspace_root) {
        return Err(format!(
            "no Git checkout to date the release; set {SOURCE_DATE_VAR}"
        ));
    }
    git(workspace_root, &["log", "-1", "--format=%ct", "HEAD"])
        .as_deref()
        .and_then(epoch_seconds)
        .map(date_of_epoch)
        .ok_or_else(|| "git log gave no commit time".to_owned())
}

/// Seconds since 1970 between year 0 and year 9999, so the date keeps four digits.
fn epoch_seconds(value: &str) -> Option<i64> {
    value
        .parse()
        .ok()
        .filter(|s| (-62_167_219_200..=253_402_300_799).contains(s))
}

/// UTC calendar date of `seconds` since 1970-01-01 (proleptic Gregorian calendar).
fn date_of_epoch(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn is_iso_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| match i {
            4 | 7 => *b == b'-',
            _ => b.is_ascii_digit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = "<?xml version=\"1.0\"?>\n<component type=\"desktop-application\">\n  <id>x</id>\n</component>\n";

    /// AC-M6-23-3 (b): the release entry for the build version is the last child of
    /// `<component>`.
    #[test]
    fn ac_m6_23_3_release_entry_is_added_for_the_version() {
        let out = with_release(TEMPLATE, "0.6.0~m6.10", "2026-11-27").expect("release added");
        assert_eq!(
            out,
            "<?xml version=\"1.0\"?>\n<component type=\"desktop-application\">\n  <id>x</id>\n  \
             <releases>\n    <release version=\"0.6.0~m6.10\" date=\"2026-11-27\"/>\n  \
             </releases>\n</component>\n"
        );
    }

    /// AC-M6-23-3 (a): a template with a hand-written release is refused.
    #[test]
    fn ac_m6_23_3_template_with_a_release_is_refused() {
        let by_hand = TEMPLATE.replace(
            "</component>",
            "<releases><release version=\"0.6.0\" date=\"2026-11-27\"/></releases></component>",
        );
        assert!(with_release(&by_hand, "0.6.0", "2026-11-27").is_err());
    }

    /// A version or date that would break the XML or the AppStream format is refused, and so is
    /// a file without `<component>`.
    #[test]
    fn invalid_input_is_refused() {
        assert!(
            with_release(TEMPLATE, "0.6.0\"/>", "2026-11-27").is_err(),
            "version"
        );
        assert!(
            with_release(TEMPLATE, "", "2026-11-27").is_err(),
            "empty version"
        );
        assert!(
            with_release(TEMPLATE, "0.6.0", "27.11.2026").is_err(),
            "date"
        );
        assert!(
            with_release("<other/>", "0.6.0", "2026-11-27").is_err(),
            "no component"
        );
    }

    /// `SOURCE_DATE_EPOCH` gives the UTC date: (a) the epoch, (b) a leap day, (c) the last second
    /// of a day, and (d) a value that is no number fails.
    #[test]
    fn source_date_epoch_gives_the_utc_date() {
        let root = Path::new("/nonexistent");
        assert_eq!(
            release_date(root, Some("0")).as_deref(),
            Ok("1970-01-01"),
            "(a)"
        );
        assert_eq!(
            release_date(root, Some("1709208000")).as_deref(),
            Ok("2024-02-29"),
            "(b)"
        );
        assert_eq!(
            release_date(root, Some("1798761599")).as_deref(),
            Ok("2026-12-31"),
            "(c)"
        );
        assert!(release_date(root, Some("yesterday")).is_err(), "(d)");
    }

    /// An empty `SOURCE_DATE_EPOCH` counts as unset, and a date beyond year 9999 is refused.
    #[test]
    fn empty_or_out_of_range_epoch() {
        let root = Path::new("/nonexistent");
        assert!(
            release_date(root, Some("")).is_err_and(|e| e.contains("no Git checkout")),
            "empty counts as unset"
        );
        assert!(
            release_date(root, Some("253402300800")).is_err(),
            "year 10000"
        );
    }

    /// Without `SOURCE_DATE_EPOCH` and outside a Git checkout there is no date to use.
    #[test]
    fn no_checkout_and_no_epoch_fails() {
        assert!(release_date(Path::new("/nonexistent"), None).is_err());
    }
}
