// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick

// Waits for the answer to one daemon request at a time: `track(request)` takes the id an
// invokable returned, `answered(ok, result)` follows once the daemon has answered it. A newer
// request replaces the older one, whose answer is then ignored; destroyed with its page, so an
// answer that arrives later reaches nobody.
Connections {
    id: answer

    // The `KesticApp` that answers the requests.
    required property var backend
    // The request waited for; 0 when none is on its way.
    property int pending: 0
    readonly property bool waiting: pending !== 0

    signal answered(bool ok, string result)

    target: backend

    // Waits for `request`; a request that failed before it was sent (0) answers at once.
    function track(request) {
        pending = request;
        if (request === 0) {
            answered(false, "");
        }
    }

    function onDaemonCallDone(request, ok, result) {
        if (request === 0 || request !== pending) return;
        pending = 0;
        answered(ok, result);
    }
}
