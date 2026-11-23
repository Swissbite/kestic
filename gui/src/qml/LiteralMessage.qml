// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import org.kde.kirigami as Kirigami

// An inline message whose text is always shown literally. The label of Kirigami.InlineMessage
// shows rich text, so `plainText` is escaped and marked as rich text: a name from a snapshot or
// the disk in the message never turns into markup.
Kirigami.InlineMessage {
    property string plainText: ""

    function markup(raw) {
        return "<span>" + raw.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
            + "</span>";
    }

    text: markup(plainText)
}
