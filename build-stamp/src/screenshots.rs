// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: BSD-2-Clause

//! The `<screenshots>` of the AppStream metainfo. They come from one list,
//! `packaging/screenshots.tsv`, and point to the assets of the package's own release, which the
//! release workflow attaches under the same fixed names. So every package keeps its own images,
//! whatever the website shows later.

use crate::version::DEV_VERSION;

/// Where the assets of a release are downloaded from; the tag and the file name follow.
pub const RELEASE_DOWNLOAD: &str = "https://github.com/Swissbite/kestic/releases/download";

/// One line of the screenshot list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screenshot {
    /// Fixed name of the release asset, e.g. `screenshot-overview.png`.
    pub name: String,
    /// File name of the GUI smoke screenshot the asset is taken from.
    pub source: String,
    /// English caption; the German one comes from the catalog.
    pub caption: String,
}

/// The entries of the screenshot list: tab-separated `name`, `source`, `caption`; lines that are
/// empty or start with `#` are skipped. Fails for a malformed line, a name that is not
/// `screenshot-<a-z, 0-9, ->.png`, a source that is no plain PNG file name, a repeated name, or an
/// empty list.
pub fn parse_list(text: &str) -> Result<Vec<Screenshot>, String> {
    let mut shots: Vec<Screenshot> = Vec::new();
    for (number, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let [name, source, caption] = fields.as_slice() else {
            return Err(format!(
                "screenshot list line {}: expected name, source and caption separated by tabs",
                number + 1
            ));
        };
        if !is_asset_name(name) {
            return Err(format!(
                "screenshot name {name:?} is not screenshot-<a-z0-9->.png"
            ));
        }
        if !is_png_file_name(source) {
            return Err(format!("screenshot source {source:?} is no PNG file name"));
        }
        if !is_plain_caption(caption) {
            return Err(format!(
                "screenshot {name}: caption must be plain text without blanks at the edges, double blanks or &, <, >"
            ));
        }
        if shots.iter().any(|s| s.name == *name) {
            return Err(format!("screenshot name {name} is listed twice"));
        }
        shots.push(Screenshot {
            name: (*name).to_owned(),
            source: (*source).to_owned(),
            caption: (*caption).to_owned(),
        });
    }
    if shots.is_empty() {
        return Err("the screenshot list is empty".to_owned());
    }
    Ok(shots)
}

/// The tag of the release of `version` whose assets carry the screenshots. `None` for a build
/// without such a release: a development build, commits after a tag (`+` in the version), and a
/// pre-release of an integration branch (`~m<n>.<k>`), which the release workflow publishes
/// without screenshots.
pub fn release_tag(version: &str) -> Option<String> {
    if version.contains('+') || version == DEV_VERSION || version.is_empty() {
        return None;
    }
    if version
        .split_once('~')
        .is_some_and(|(_, pre)| is_integration_pre_release(pre))
    {
        return None;
    }
    Some(format!("v{}", version.replace('~', "-")))
}

/// `m<n>.<k>`, the pre-release part of a build of an integration branch.
fn is_integration_pre_release(pre: &str) -> bool {
    pre.strip_prefix('m')
        .and_then(|rest| rest.split_once('.'))
        .is_some_and(|(n, k)| {
            [n, k]
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        })
}

/// `metainfo` with a `<screenshots>` block for `shots` added as the last child of `<component>`,
/// the images being assets of the release of `version`. Unchanged for a version without its own
/// release. Fails for a template that already has screenshots: the list is their only source.
pub fn with_screenshots(
    metainfo: &str,
    shots: &[Screenshot],
    version: &str,
) -> Result<String, String> {
    if metainfo.contains("<screenshot") {
        return Err(
            "the metainfo template must not contain screenshots: they come from packaging/screenshots.tsv"
                .to_owned(),
        );
    }
    let Some(tag) = release_tag(version) else {
        return Ok(metainfo.to_owned());
    };
    let end = metainfo
        .rfind("</component>")
        .ok_or("the metainfo has no </component>")?;
    let mut block = String::from("  <screenshots>\n");
    for (i, shot) in shots.iter().enumerate() {
        block.push_str(if i == 0 {
            "    <screenshot type=\"default\">\n"
        } else {
            "    <screenshot>\n"
        });
        block.push_str(&format!(
            "      <caption>{}</caption>\n      <image>{RELEASE_DOWNLOAD}/{tag}/{}</image>\n    </screenshot>\n",
            shot.caption,
            shot.name
        ));
    }
    block.push_str("  </screenshots>\n");
    Ok(format!("{}{block}{}", &metainfo[..end], &metainfo[end..]))
}

fn is_asset_name(name: &str) -> bool {
    name.strip_prefix("screenshot-")
        .and_then(|rest| rest.strip_suffix(".png"))
        .is_some_and(|stem| {
            !stem.is_empty()
                && stem
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

fn is_png_file_name(name: &str) -> bool {
    name.strip_suffix(".png").is_some_and(|stem| {
        !stem.is_empty()
            && !stem.starts_with('.')
            && stem
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    })
}

/// A caption that goes into the XML as it is and that msgfmt matches with its catalog entry:
/// no markup characters, no blanks at the edges, no runs of whitespace.
fn is_plain_caption(caption: &str) -> bool {
    !caption.is_empty()
        && caption.trim() == caption
        && !caption.contains("  ")
        && !caption.contains(['&', '<', '>', '\t', '\r', '\n'])
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "# comment\n\
        screenshot-overview.png\tm3-overview-status-wide-en.png\tOverview and status\n\
        screenshot-restore.png\tm4-restore-browse-wide-en.png\tRestore\n";
    const TEMPLATE: &str = "<component>\n  <id>x</id>\n</component>\n";

    /// AC-M6-23-4 (a): a release (b) and a release candidate (c) link the release assets of
    /// their own tag; a development build (d), commits after a tag (e) and a pre-release of an
    /// integration branch (f), published without screenshots, link none.
    #[test]
    fn ac_m6_23_4_release_tag_of_the_own_version() {
        assert_eq!(release_tag("0.6.0").as_deref(), Some("v0.6.0"), "(b)");
        assert_eq!(
            release_tag("1.0.0~rc.1").as_deref(),
            Some("v1.0.0-rc.1"),
            "(c)"
        );
        assert_eq!(release_tag("0.6.0~m6.11"), None, "(f)");
        assert_eq!(release_tag("0.0.0~dev"), None, "(d)");
        assert_eq!(release_tag("0.0.0~dev+gabc1234"), None, "(d) with commit");
        assert_eq!(release_tag("0.6.0~m6.10+3.gabc1234"), None, "(e)");
    }

    /// AC-M6-23-4 (a), (b): the block lists the screenshots in list order, the first one as
    /// default, each image a release asset of the version's tag under its fixed name.
    #[test]
    fn ac_m6_23_4_screenshots_point_to_release_assets() {
        let shots = parse_list(LIST).expect("list");
        let out = with_screenshots(TEMPLATE, &shots, "0.6.0~rc.2").expect("screenshots");
        assert_eq!(
            out,
            "<component>\n  <id>x</id>\n  <screenshots>\n    <screenshot type=\"default\">\n      \
             <caption>Overview and status</caption>\n      <image>https://github.com/Swissbite/kestic/releases/download/v0.6.0-rc.2/screenshot-overview.png</image>\n    \
             </screenshot>\n    <screenshot>\n      <caption>Restore</caption>\n      \
             <image>https://github.com/Swissbite/kestic/releases/download/v0.6.0-rc.2/screenshot-restore.png</image>\n    \
             </screenshot>\n  </screenshots>\n</component>\n"
        );
    }

    /// A build without its own release ships no screenshots, and a template with screenshots of
    /// its own is refused.
    #[test]
    fn no_release_no_screenshots_and_none_by_hand() {
        let shots = parse_list(LIST).expect("list");
        assert_eq!(
            with_screenshots(TEMPLATE, &shots, "0.6.0+2.gabc1234").as_deref(),
            Ok(TEMPLATE)
        );
        let by_hand = TEMPLATE.replace("</component>", "<screenshots/></component>");
        assert!(with_screenshots(&by_hand, &shots, "0.6.0").is_err());
    }

    /// AC-M6-23-4 (b): malformed lists are refused: (a) two fields, (b) a name outside the fixed
    /// pattern, (c) a source with a path, (d) a repeated name, (e) a caption that is empty or not
    /// plain text, (f) no entry, (g) four fields. A list with CRLF line ends or without a final
    /// newline is read completely.
    #[test]
    fn ac_m6_23_4_malformed_lists_are_refused() {
        assert!(parse_list("screenshot-a.png\ta.png\n").is_err(), "(a)");
        assert!(parse_list("Overview.png\ta.png\tCaption\n").is_err(), "(b)");
        assert!(
            parse_list("screenshot-a.png\t../a.png\tCaption\n").is_err(),
            "(c)"
        );
        assert!(
            parse_list("screenshot-a.png\ta.png\tA\nscreenshot-a.png\tb.png\tB\n").is_err(),
            "(d)"
        );
        assert!(parse_list("screenshot-a.png\ta.png\t \n").is_err(), "(e)");
        assert!(
            parse_list("screenshot-a.png\ta.png\tA & B\n").is_err(),
            "(e) markup"
        );
        assert!(
            parse_list("screenshot-a.png\ta.png\tA  B\n").is_err(),
            "(e) double blank"
        );
        assert!(parse_list("# only a comment\n").is_err(), "(f)");
        assert!(
            parse_list("screenshot-a.png\ta.png\tA\tB\n").is_err(),
            "(g)"
        );
        let read = parse_list("screenshot-a.png\ta.png\tA\r\nscreenshot-b.png\tb.png\tB")
            .expect("CRLF, no final newline");
        assert_eq!(read.len(), 2, "every line read");
        assert_eq!(read[0].caption, "A", "no CR in the caption");
    }
}
