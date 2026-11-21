// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

// A row of a detail page that opens a subpage: icon, one line of text and the «go next»
// arrow. The caller sets `objectName`, `onClicked` and the three properties below.
Controls.ItemDelegate {
    id: navRow

    required property string accessibleName
    required property string iconSource
    required property string labelText

    Layout.fillWidth: true
    activeFocusOnTab: true
    Accessible.name: accessibleName

    FocusRing {
        target: navRow
    }

    contentItem: RowLayout {
        spacing: Kirigami.Units.smallSpacing
        Kirigami.Icon {
            source: navRow.iconSource
            Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
            Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
        }
        Controls.Label {
            text: navRow.labelText
            elide: Text.ElideRight
            Layout.fillWidth: true
        }
        Kirigami.Icon {
            source: "go-next"
            Layout.preferredWidth: Kirigami.Units.iconSizes.small
            Layout.preferredHeight: Kirigami.Units.iconSizes.small
        }
    }
}
