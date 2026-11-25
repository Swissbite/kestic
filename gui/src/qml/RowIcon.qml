// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import org.kde.kirigami as Kirigami

// A coloured icon on a list row. On the selected row it sits on the background colour: an
// icon in the colour of the selection would otherwise lose its body. The backdrop lies within
// the item (`frame` around the icon), so the highlight shows between it and a focus ring.
Item {
    id: rowIcon

    property alias source: icon.source
    property int iconSize: Kirigami.Units.iconSizes.small
    // Whether the row is selected (highlighted).
    property bool onSelection: false
    property alias backdropObjectName: backdrop.objectName
    readonly property int frame: 2

    implicitWidth: iconSize + 2 * frame
    implicitHeight: iconSize + 2 * frame

    Rectangle {
        id: backdrop
        visible: rowIcon.onSelection
        anchors.fill: parent
        radius: Kirigami.Units.cornerRadius
        color: Kirigami.Theme.backgroundColor
    }

    Kirigami.Icon {
        id: icon
        anchors.centerIn: parent
        width: rowIcon.iconSize
        height: rowIcon.iconSize
    }
}
