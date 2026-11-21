// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import org.kde.kirigami as Kirigami

// The keyboard focus ring over a control: declare it as a child of the control and set
// `target` to the control. It is only a border, so it never hides the text beneath it.
Rectangle {
    id: ring

    required property Item target
    // Whether the ring shows; a control with a different notion of focus overrides it.
    property bool focused: Boolean(target.activeFocus || target.visualFocus)
    property color ringColor: Kirigami.Theme.highlightColor

    anchors.fill: parent
    z: 10
    color: "transparent"
    radius: Kirigami.Units.cornerRadius
    border.width: focused ? 2 : 0
    border.color: ringColor
    visible: border.width > 0
}
