// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick

// A list that is one Tab stop: Tab enters it on its current row, Up/Down, Home/End and
// Page Up/Page Down move within it (End and Page Down load the rows of a paged model they need),
// and the next Tab or Shift+Tab leaves it. Its rows take
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
        if (activeFocus && count > 0 && currentIndex < 0) moveCurrentTo(0, 1);
    }

    // Whether row `i` can be the current one; a list whose rows are not all selectable
    // defines its own `selectable(i)`.
    function canBeCurrent(i) {
        return typeof list.selectable !== "function" || list.selectable(i);
    }

    // Makes row `index` (kept within the list) the current one and scrolls to it; a row that
    // cannot be current is passed over in the direction `step` (+1 or -1), or the other way at
    // the end of the list.
    function moveCurrentTo(index, step) {
        if (count === 0) return;
        var i = Math.max(0, Math.min(count - 1, index));
        var dir = step < 0 ? -1 : 1;
        while (i >= 0 && i < count && !canBeCurrent(i)) i += dir;
        if (i < 0 || i >= count) {
            i = Math.max(0, Math.min(count - 1, index));
            while (i >= 0 && i < count && !canBeCurrent(i)) i -= dir;
        }
        if (i < 0 || i >= count) return;
        currentIndex = i;
        positionViewAtIndex(currentIndex, ListView.Contain);
    }

    // Loads further rows of a model that holds them in pages, until row `index` is loaded or the
    // model has none left.
    function loadRowsUpTo(index) {
        var m = list.model;
        if (!m || typeof m.canFetchMore !== "function") return;
        var root = m.index(-1, 0);
        while (index >= list.count && m.canFetchMore(root)) {
            var before = list.count;
            m.fetchMore(root);
            if (list.count === before) return;
        }
    }

    // How many rows of the current row's height fit into the list's height, at least one.
    function rowsPerPage() {
        var row = currentItem ? currentItem.height : 0;
        return row > 0 ? Math.max(1, Math.floor(height / row)) : 1;
    }

    Keys.onPressed: function(event) {
        if (event.modifiers !== Qt.NoModifier && event.modifiers !== Qt.KeypadModifier) return;
        // Up and Down move here rather than by the view's own key navigation, which does not
        // keep the row clear of a header that stays above the rows; at the first or last row
        // the key passes on.
        if (event.key === Qt.Key_Up && list.currentIndex > 0) {
            list.moveCurrentTo(list.currentIndex - 1, -1);
        } else if (event.key === Qt.Key_Down && list.currentIndex < 0 && list.count > 0) {
            list.moveCurrentTo(0, 1);
        } else if (event.key === Qt.Key_Down && list.currentIndex >= 0) {
            list.loadRowsUpTo(list.currentIndex + 1);
            if (list.currentIndex + 1 >= list.count) return;
            list.moveCurrentTo(list.currentIndex + 1, 1);
        } else if (event.key === Qt.Key_Home) {
            list.moveCurrentTo(0, 1);
        } else if (event.key === Qt.Key_End) {
            list.loadRowsUpTo(Number.MAX_SAFE_INTEGER);
            list.moveCurrentTo(list.count - 1, -1);
        } else if (event.key === Qt.Key_PageUp) {
            list.moveCurrentTo(list.currentIndex - list.rowsPerPage(), -1);
        } else if (event.key === Qt.Key_PageDown) {
            var target = list.currentIndex + list.rowsPerPage();
            list.loadRowsUpTo(target);
            list.moveCurrentTo(target, 1);
        } else {
            return;
        }
        event.accepted = true;
    }
}
