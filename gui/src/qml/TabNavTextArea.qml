// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls

// A text area that Tab and Shift+Tab leave instead of inserting a tab character.
Controls.TextArea {
    Keys.onTabPressed: function(event) {
        var next = nextItemInFocusChain(true);
        if (next) next.forceActiveFocus(Qt.TabFocusReason);
        event.accepted = true;
    }
    Keys.onBacktabPressed: function(event) {
        var prev = nextItemInFocusChain(false);
        if (prev) prev.forceActiveFocus(Qt.BacktabFocusReason);
        event.accepted = true;
    }
}
