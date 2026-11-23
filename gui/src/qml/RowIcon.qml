// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import org.kde.kirigami as Kirigami

// A coloured icon on a list row. On the selected row it sits on the background colour: an
// icon in the colour of the selection would otherwise lose its body.
Item {
    id: rowIcon

    property alias source: icon.source
    // Whether the row is selected (highlighted).
    property bool onSelection: false
    property alias backdropObjectName: backdrop.objectName

    Rectangle {
        id: backdrop
        visible: rowIcon.onSelection
        anchors.fill: parent
        anchors.margins: -2
        radius: Kirigami.Units.cornerRadius
        color: Kirigami.Theme.backgroundColor
    }

    Kirigami.Icon {
        id: icon
        anchors.fill: parent
    }
}
