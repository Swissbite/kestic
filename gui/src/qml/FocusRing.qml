// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import org.kde.kirigami as Kirigami

// The keyboard focus ring over a control: declare it as a child of the control and set
// `target` to the control. It is only a border, so it never hides the text beneath it.
// It follows the control's background: list rows inset their highlight and hover background
// from the row's edges (unevenly at top and bottom), and the ring lies on that background.
// On a highlighted control it is drawn in the selection's text colour, inside the highlight.
Rectangle {
    id: ring

    required property Item target
    // Whether the ring shows; a control with a different notion of focus overrides it.
    property bool focused: Boolean(target.activeFocus || target.visualFocus)
    readonly property bool onSelection: Boolean(target.highlighted)
    property color ringColor: onSelection ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.highlightColor
    // The control's background while it is a sibling of the ring; a text area in a scroll view
    // hands its background to the view, and the ring then covers the whole control.
    readonly property Item frame: target.background && target.background.parent === target
        ? target.background : null

    anchors.fill: frame ? frame : parent
    anchors.margins: onSelection ? 1 : 0
    z: 10
    color: "transparent"
    radius: Kirigami.Units.cornerRadius
    border.width: focused ? 2 : 0
    border.color: ringColor
    visible: border.width > 0
}
