// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import net.daester.kestic 1.0

// Fields, checks and connection test of a destination that is being added or edited.
QtObject {

    required property var appBackend
    property string destinationId: ""
    property string kind: "local"
    // Card chosen in the step «destination type» (`drive`, `folder`, `rest`, …).
    property string typeCard: "folder"
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
    // Warning of the last successful connection test (a folder on a CIFS/SMB share) and whether
    // the user confirmed it.
    property string testNote: ""
    property bool testNoteConfirmed: false

    readonly property bool kindIsRest: kind === "rest"
    readonly property bool nameValid: name.trim().length > 0
    readonly property bool locationValid: kindIsRest
        ? /^https?:\/\//i.test(location.trim())
        : location.trim().length > 0
    readonly property bool detailsValid: nameValid && locationValid
    // The connection test passed and its warning, if any, is confirmed.
    readonly property bool connectionAccepted: connectionVerified && (testNote === "" || testNoteConfirmed)
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
        testNote = "";
        testNoteConfirmed = false;
        if (!detailsValid) {
            showError(Texts.label("fill_required_fields"));
            return false;
        }
        var ok = appBackend.testDestination(fieldsJson(), setupMode, repoPassword, restPassword);
        testNote = ok ? appBackend.connection_test_note : "";
        connectionVerified = ok;
        testFeedbackOk = ok;
        testFeedback = appBackend.connection_test_message;
        return ok;
    }
}
