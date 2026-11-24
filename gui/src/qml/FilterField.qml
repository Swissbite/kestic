// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// The field «Filter» of a list: filters by text in the path once typing pauses for 300 ms.
// Ctrl+F belongs to the search field of the snapshot list, so this field has no shortcut.
Kirigami.SearchField {
    id: field

    // The text to filter by, once typing has paused.
    signal filterRequested(string text)

    // Empties the field for a list that starts unfiltered.
    function resetFilter() {
        delay.stop();
        text = "";
    }

    placeholderText: Texts.label("filter")
    Accessible.name: Texts.label("filter")
    focusSequences: []
    onTextChanged: delay.restart()

    Timer {
        id: delay
        interval: 300
        onTriggered: field.filterRequested(field.text)
    }
}
