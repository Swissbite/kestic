// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import org.kde.kirigami as Kirigami
import org.kde.kirigami.delegates as KD

// A list row with an icon, a title and a subtitle below it. The title has the default font like
// every other label; the subtitle has the small font. Where the platform reports a small font
// larger than the default one (no Plasma platform theme), the subtitle gets four fifths of the
// title's size, as Kirigami's own small font without a platform theme.
KD.SubtitleDelegate {
    id: row

    readonly property font subtitleFont: Kirigami.Theme.smallFont.pointSize > 0 && font.pointSize > 0
        && Kirigami.Theme.smallFont.pointSize > font.pointSize
        ? Qt.font({ family: font.family, pointSize: font.pointSize * 0.8 })
        : Kirigami.Theme.smallFont

    font: Kirigami.Theme.defaultFont

    contentItem: KD.IconTitleSubtitle {
        icon: icon.fromControlsIcon(row.icon)
        title: row.text
        subtitle: row.subtitle
        selected: row.highlighted || row.down
        font: row.font
        subtitleFont: row.subtitleFont
    }
}
