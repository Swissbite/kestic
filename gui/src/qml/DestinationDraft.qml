// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import net.daester.kestic 1.0

// Fields, checks and connection test of a destination that is being added or edited.
QtObject {

    required property var appBackend
    property string destinationId: ""
    property string kind: "local"
    property string setupMode: "init"
    property string name: ""
    property string location: ""
    property string username: ""
    property string restPassword: ""
    property string repoPassword: ""
    property string repoPasswordConfirm: ""
    property bool connectionVerified: false
    property string testFeedback: ""
    property bool testFeedbackOk: false

    readonly property bool kindIsRest: kind === "rest"
    readonly property bool nameValid: name.trim().length > 0
    readonly property bool locationValid: kindIsRest
        ? /^https?:\/\//i.test(location.trim())
        : location.trim().length > 0
    readonly property bool detailsValid: nameValid && locationValid
    readonly property bool passwordsMatch: repoPassword.length > 0 && (setupMode === "connect" || repoPassword === repoPasswordConfirm)
    readonly property bool hasAnyInput: name.trim().length > 0 || location.trim().length > 0 || repoPassword.length > 0

    function fieldsJson() {
        var fields = {
            kind: kind,
            name: name.trim(),
            location: location.trim(),
            username: username.trim()
        };
        if (destinationId !== "") {
            fields.id = destinationId;
        }
        return JSON.stringify(fields);
    }

    function showError(text) {
        testFeedbackOk = false;
        testFeedback = text;
    }

    function runConnectionTest() {
        if (!detailsValid) {
            showError(Texts.label("fill_required_fields"));
            return false;
        }
        var ok = appBackend.testDestination(fieldsJson(), setupMode, repoPassword, restPassword);
        connectionVerified = ok;
        testFeedbackOk = ok;
        testFeedback = appBackend.connection_test_message;
        return ok;
    }
}
