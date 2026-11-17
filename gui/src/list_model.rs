// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Spike QAbstractListModel exposed to QML via CXX-Qt (M0-S04).

use crate::models::qobject;
use cxx_kde_frameworks::ki18n::i18n;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QVariant};

/// Rust backing data for `SpikeListModel` with three entries (AC-M0-04-1, AC-M0-09-1).
pub struct SpikeListModelRust {
    items: Vec<(&'static str, &'static str)>,
}

impl Default for SpikeListModelRust {
    fn default() -> Self {
        Self {
            items: vec![
                ("Personal folder", "~ -> NAS · backed up 2 hours ago"),
                ("Photos", "/media/photos -> USB-Disk · waiting"),
                ("Projects", "~/Projects -> S3 · password rejected"),
            ],
        }
    }
}

impl qobject::SpikeListModel {
    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let row = index.row();
        if row < 0 {
            return QVariant::default();
        }
        let Some((title, subtitle)) = self.items.get(row as usize) else {
            return QVariant::default();
        };

        // Call i18n directly in data() after KLocalizedString::set_application_domain("kestic")
        // is initialized (AC-M0-09-1). Keep i18n literals extractable for xgettext below:
        let localized_title = match *title {
            "Personal folder" => i18n("Personal folder"),
            "Photos" => i18n("Photos"),
            "Projects" => i18n("Projects"),
            other => i18n(other),
        };
        let localized_subtitle = match *subtitle {
            "~ -> NAS · backed up 2 hours ago" => i18n("~ -> NAS · backed up 2 hours ago"),
            "/media/photos -> USB-Disk · waiting" => i18n("/media/photos -> USB-Disk · waiting"),
            "~/Projects -> S3 · password rejected" => i18n("~/Projects -> S3 · password rejected"),
            other => i18n(other),
        };

        match (qobject::Roles { repr: role }) {
            qobject::Roles::Title => QVariant::from(&localized_title),
            qobject::Roles::Subtitle => QVariant::from(&localized_subtitle),
            _ => QVariant::default(),
        }
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(qobject::Roles::Title.repr, QByteArray::from("title"));
        roles.insert(qobject::Roles::Subtitle.repr, QByteArray::from("subtitle"));
        roles
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.items.len() as i32
    }
}
