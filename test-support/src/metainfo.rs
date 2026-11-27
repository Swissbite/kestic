// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! A small reader for the AppStream metainfo of Kestic (template and shipped file): the elements
//! of one tag with their attributes and text. Enough for the flat, generated XML of that file;
//! no general XML parser.

/// The template in the repository, relative to the workspace root.
pub const TEMPLATE: &str = "packaging/net.daester.Kestic.metainfo.xml";

/// Path of the metainfo inside the `.deb`.
pub const SHIPPED: &str = "usr/share/metainfo/net.daester.Kestic.metainfo.xml";

/// One element: the text between its start tag and its end tag (empty for `<tag/>`), and the
/// raw attributes of the start tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    attrs: String,
    pub text: String,
}

impl Element {
    /// Value of the attribute `name`, if the start tag has it.
    pub fn attr(&self, name: &str) -> Option<&str> {
        let start = self.attrs.find(&format!("{name}=\""))? + name.len() + 2;
        let len = self.attrs[start..].find('"')?;
        Some(&self.attrs[start..start + len])
    }

    /// The `xml:lang` of the element, `None` for the untranslated one.
    pub fn lang(&self) -> Option<&str> {
        self.attr("xml:lang")
    }
}

/// All elements `<tag …>` in `xml`, in document order. Nested elements of the same tag are not
/// supported.
pub fn elements(xml: &str, tag: &str) -> Vec<Element> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(pos) = rest.find(&open) {
        let after = &rest[pos + open.len()..];
        if !after.starts_with([' ', '>', '/', '\n']) {
            rest = after;
            continue;
        }
        let end_of_start = after.find('>').expect("start tag ends");
        let start_tag = &after[..end_of_start];
        let body = &after[end_of_start + 1..];
        if let Some(attrs) = start_tag.strip_suffix('/') {
            found.push(Element {
                attrs: attrs.to_owned(),
                text: String::new(),
            });
            rest = body;
            continue;
        }
        let end = body.find(&close).expect("element has an end tag");
        found.push(Element {
            attrs: start_tag.to_owned(),
            text: unescape(&body[..end]),
        });
        rest = &body[end + close.len()..];
    }
    found
}

/// The untranslated text of the first `<tag>` in `xml`.
pub fn text(xml: &str, tag: &str) -> Option<String> {
    elements(xml, tag)
        .into_iter()
        .find(|e| e.lang().is_none())
        .map(|e| e.text)
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}
