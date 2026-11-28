// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick

// A list that is one Tab stop: Tab enters it on its current row, Up/Down, Home/End and
// Page Up/Page Down move within it (Down, End and Page Down load the rows of a paged model they
// need and move on once they have arrived),
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
    // Leaving the list drops a move still waiting for rows.
    onActiveFocusChanged: {
        if (activeFocus) Qt.callLater(list.enterFirstRowIfNone);
        else pendingRow = -1;
    }
    // A page asked for by a key arrived: on to the next one, or to the row.
    onCountChanged: {
        if (activeFocus) Qt.callLater(list.enterFirstRowIfNone);
        if (pendingRow >= 0) Qt.callLater(list.continuePendingMove);
    }

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
        movingForKey = true;
        currentIndex = i;
        movingForKey = false;
        positionViewAtIndex(currentIndex, ListView.Contain);
        keepCurrentRowInOuterView();
    }

    // The nearest flickable around a list that does not scroll itself (one as tall as its rows,
    // in a scrollable page); null when the list scrolls itself or nothing around it scrolls.
    function outerFlickable() {
        if (interactive) return null;
        for (var p = parent; p; p = p.parent) {
            if (p instanceof Flickable) return p;
        }
        return null;
    }

    // Scrolls the flickable around the list just so far that the current row is in view.
    function keepCurrentRowInOuterView() {
        var flick = outerFlickable();
        if (!flick || !currentItem) return;
        var top = currentItem.mapToItem(flick.contentItem, 0, 0).y;
        var bottom = top + currentItem.height;
        var y = flick.contentY;
        if (bottom > y + flick.height) y = bottom - flick.height;
        if (top < y) y = top;
        var minY = flick.originY - flick.topMargin;
        var maxY = Math.max(minY, flick.originY + flick.contentHeight + flick.bottomMargin - flick.height);
        flick.contentY = Math.max(minY, Math.min(maxY, y));
    }

    // The row a key asked for that is not loaded yet (-1: none) and the direction of the move;
    // the list moves there once the rows have arrived.
    property real pendingRow: -1
    property int pendingStep: 1
    // The current row changes for a key (not a click); another change drops a waiting move.
    property bool movingForKey: false
    onCurrentIndexChanged: if (!movingForKey) pendingRow = -1

    // Loads further rows of a model that holds them in pages, one page after the other, until
    // row `index` is loaded or the model has none left; then the list moves there (`step`: the
    // direction, as for `moveCurrentTo`). Returns whether the row is loaded already.
    function loadRowsUpTo(index, step) {
        var m = list.model;
        var root = m && typeof m.canFetchMore === "function" ? m.index(-1, 0) : null;
        if (index < list.count || !root || !m.canFetchMore(root)) {
            list.pendingRow = -1;
            return true;
        }
        list.pendingRow = index;
        list.pendingStep = step;
        m.fetchMore(root);
        return false;
    }

    function continuePendingMove() {
        if (pendingRow < 0) return;
        var target = pendingRow;
        if (loadRowsUpTo(target, pendingStep)) {
            userMove = true;
            moveCurrentTo(Math.min(target, count - 1), pendingStep);
            userMove = false;
        }
    }

    // How many rows of the current row's height fit into the part of the list that can be seen
    // at once (the list, or the flickable around it when the list does not scroll), at least one.
    function rowsPerPage() {
        var row = currentItem ? currentItem.height : 0;
        var flick = outerFlickable();
        var seen = flick ? Math.min(height, flick.height) : height;
        return row > 0 ? Math.max(1, Math.floor(seen / row)) : 1;
    }

    Keys.onPressed: function(event) {
        if (event.modifiers !== Qt.NoModifier && event.modifiers !== Qt.KeypadModifier) return;
        list.userMove = true;
        list.moveForKey(event);
        list.userMove = false;
    }

    // While true the current row changes because the user pressed a key (also once the rows a
    // key waited for have arrived); a page can tell such a move from one of its own.
    property bool userMove: false

    function moveForKey(event) {
        // Another key than one that waits for rows drops a move still waiting for them.
        if (event.key !== Qt.Key_Down && event.key !== Qt.Key_End && event.key !== Qt.Key_PageDown) {
            list.pendingRow = -1;
        }
        // Up and Down move here rather than by the view's own key navigation, which does not
        // keep the row clear of a header that stays above the rows; at the first or last row
        // the key passes on.
        if (event.key === Qt.Key_Up && list.currentIndex > 0) {
            list.moveCurrentTo(list.currentIndex - 1, -1);
        } else if (event.key === Qt.Key_Down && list.currentIndex < 0 && list.count > 0) {
            list.moveCurrentTo(0, 1);
        } else if (event.key === Qt.Key_Down && list.currentIndex >= 0) {
            if (list.currentIndex + 1 >= list.count && list.loadRowsUpTo(list.currentIndex + 1, 1)) return;
            if (list.currentIndex + 1 < list.count) list.moveCurrentTo(list.currentIndex + 1, 1);
        } else if (event.key === Qt.Key_Home) {
            list.pendingRow = -1;
            list.moveCurrentTo(0, 1);
        } else if (event.key === Qt.Key_End) {
            // The last loaded row at once; the last row once the rows have arrived.
            list.loadRowsUpTo(Number.MAX_SAFE_INTEGER, -1);
            list.moveCurrentTo(list.count - 1, -1);
        } else if (event.key === Qt.Key_PageUp) {
            list.pendingRow = -1;
            list.moveCurrentTo(list.currentIndex - list.rowsPerPage(), -1);
        } else if (event.key === Qt.Key_PageDown) {
            var target = list.currentIndex + list.rowsPerPage();
            list.loadRowsUpTo(target, 1);
            list.moveCurrentTo(target, 1);
        } else {
            return;
        }
        event.accepted = true;
    }
}
