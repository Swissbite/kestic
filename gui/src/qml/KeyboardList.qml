// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick

// A list that is one Tab stop: Tab enters it on its current row, Up/Down, Home/End and
// Page Up/Page Down move within it, and the next Tab or Shift+Tab leaves it. Its rows take
// the focus only by a click (`focusPolicy: Qt.ClickFocus`), so they are no Tab stops of their
// own; the list gives the focus to its current row.
ListView {
    id: list

    activeFocusOnTab: visible && count > 0
    keyNavigationEnabled: true

    // Without a current row the list holds the focus itself; the first row takes it then, also
    // when the rows arrive while the list has the focus. Runs after the list's own handler,
    // which may choose another row first.
    onActiveFocusChanged: if (activeFocus) Qt.callLater(list.enterFirstRowIfNone)
    onCountChanged: if (activeFocus) Qt.callLater(list.enterFirstRowIfNone)

    function enterFirstRowIfNone() {
        if (activeFocus && count > 0 && currentIndex < 0) currentIndex = 0;
    }

    // Makes row `index` (kept within the list) the current one and scrolls to it.
    function moveCurrentTo(index) {
        if (count === 0) return;
        currentIndex = Math.max(0, Math.min(count - 1, index));
        positionViewAtIndex(currentIndex, ListView.Contain);
    }

    // How many rows of the current row's height fit into the list's height, at least one.
    function rowsPerPage() {
        var row = currentItem ? currentItem.height : 0;
        return row > 0 ? Math.max(1, Math.floor(height / row)) : 1;
    }

    Keys.onPressed: function(event) {
        if (event.modifiers !== Qt.NoModifier && event.modifiers !== Qt.KeypadModifier) return;
        if (event.key === Qt.Key_Home) {
            list.moveCurrentTo(0);
        } else if (event.key === Qt.Key_End) {
            list.moveCurrentTo(list.count - 1);
        } else if (event.key === Qt.Key_PageUp) {
            list.moveCurrentTo(list.currentIndex - list.rowsPerPage());
        } else if (event.key === Qt.Key_PageDown) {
            list.moveCurrentTo(list.currentIndex + list.rowsPerPage());
        } else {
            return;
        }
        event.accepted = true;
    }
}
