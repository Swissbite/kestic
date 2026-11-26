// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kirigami.delegates as KD

// A list row with an icon, a title and a subtitle below it, laid out like Kirigami's
// IconTitleSubtitle. Both texts are plain text: a title can come from outside (the label of a
// drive) and is never read as markup. The title has the default font like every other label; the
// subtitle has the small font. Where the platform reports a small font larger than the default
// one (no Plasma platform theme), the subtitle gets four fifths of the title's size, as Kirigami's
// own small font without a platform theme.
KD.SubtitleDelegate {
    id: row

    readonly property bool shownSelected: highlighted || down
    readonly property font subtitleFont: Kirigami.Theme.smallFont.pointSize > 0 && font.pointSize > 0
        && Kirigami.Theme.smallFont.pointSize > font.pointSize
        ? Qt.font({ family: font.family, pointSize: font.pointSize * 0.8 })
        : Kirigami.Theme.smallFont

    font: Kirigami.Theme.defaultFont

    contentItem: RowLayout {
        spacing: Kirigami.Units.mediumSpacing

        // A coloured icon keeps its body on the selected row (see RowIcon).
        RowIcon {
            source: row.icon.name.length > 0 ? row.icon.name : row.icon.source
            iconSize: row.subtitle.length > 0 ? Kirigami.Units.iconSizes.medium : Kirigami.Units.iconSizes.smallMedium
            onSelection: row.shownSelected
            backdropObjectName: "iconBackdrop_" + row.objectName
            Layout.alignment: Qt.AlignVCenter
        }

        ColumnLayout {
            spacing: 0
            Layout.fillWidth: true

            Controls.Label {
                text: row.text
                textFormat: Text.PlainText
                font: row.font
                elide: Text.ElideRight
                color: row.shownSelected ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                Accessible.ignored: true
                Layout.fillWidth: true
            }

            Controls.Label {
                visible: text.length > 0
                text: row.subtitle
                textFormat: Text.PlainText
                font: row.subtitleFont
                elide: Text.ElideRight
                color: row.shownSelected
                    ? Kirigami.Theme.highlightedTextColor
                    : Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.textColor, Kirigami.Theme.backgroundColor, 0.25)
                Accessible.ignored: true
                Layout.fillWidth: true
            }
        }
    }
}
