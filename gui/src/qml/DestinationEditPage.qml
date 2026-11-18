// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "destinationEditPage"
    title: i18n("Edit Connection")

    required property var rootWindow
    required property var appBackend
    required property string destinationId

    property var initialData: null
    property string destName: ""
    property string location: ""
    property string setupMode: "connect"
    property string username: ""
    property string repoPassword: ""
    property string restPassword: ""
    property bool connectionVerified: false
    property string testFeedback: ""
    property bool testFeedbackOk: false

    function loadInitialData() {
        var data = rootWindow.findDestinationData(destinationId);
        initialData = data;
        if (data) {
            destName = data.name;
            location = data.location;
            username = data.username;
        }
    }

    Component.onCompleted: loadInitialData()

    readonly property bool kindIsRest: initialData !== null && initialData.kindName === "rest"
    readonly property bool locationChanged: initialData !== null && (
        location.trim() !== initialData.location
        || username.trim() !== initialData.username
        || repoPassword.length > 0
        || restPassword.length > 0
    )
    readonly property bool needsVerification: locationChanged || setupMode === "init"
    readonly property bool dirty: initialData !== null && (
        destName.trim() !== initialData.name || needsVerification
    )
    readonly property bool nameValid: destName.trim().length > 0
    readonly property bool locationValid: kindIsRest
        ? /^https?:\/\//i.test(location.trim())
        : location.trim().length > 0
    readonly property bool fieldsValid: nameValid && locationValid
    readonly property bool canSave: dirty && fieldsValid && (!needsVerification || connectionVerified) && appBackend.blocking_kind === ""

    onDirtyChanged: appBackend.has_unsaved_changes = dirty

    onBackRequested: function(event) {
        if (dirty) {
            event.accepted = true;
            unsavedDialog.open();
        }
    }

    function handleEscape() {
        if (unsavedDialog.opened || unsavedDialog.visible) {
            unsavedDialog.close();
            return true;
        }
        if (dirty) {
            unsavedDialog.open();
            return true;
        }
        return false;
    }

    function runConnectionTest() {
        if (!fieldsValid || !initialData) {
            testFeedbackOk = false;
            testFeedback = i18n("Please fill in all required fields.");
            return false;
        }
        var fieldsJson = JSON.stringify({
            id: destinationId,
            kind: initialData.kindName,
            name: destName.trim(),
            location: location.trim(),
            username: username.trim()
        });
        var ok = appBackend.testDestination(
            fieldsJson,
            page.setupMode,
            repoPassword,
            restPassword
        );
        connectionVerified = ok;
        testFeedbackOk = ok;
        testFeedback = appBackend.connection_test_message;
        return ok;
    }

    function saveChanges() {
        if (!canSave || !initialData) return false;
        var fieldsJson = JSON.stringify({
            id: destinationId,
            kind: initialData.kindName,
            name: destName.trim(),
            location: location.trim(),
            username: username.trim()
        });
        if (appBackend.updateDestination(
                destinationId,
                fieldsJson,
                page.setupMode,
                repoPassword,
                restPassword,
                page.needsVerification
            )) {
            initialData = rootWindow.findDestinationData(destinationId);
            setupMode = "connect";
            repoPassword = "";
            restPassword = "";
            connectionVerified = false;
            testFeedback = "";
            testFeedbackOk = false;
            rootWindow.showPassiveNotification(i18n("Destination saved"));
            return true;
        }
        testFeedbackOk = false;
        testFeedback = appBackend.last_action_error;
        return false;
    }

    actions: [
        Kirigami.Action {
            id: saveDestinationAction
            objectName: "saveDestinationAction"
            icon.name: "document-save"
            text: i18n("Save")
            enabled: page.canSave
            onTriggered: page.saveChanges()
        },
        Kirigami.Action {
            id: closeDestinationEditAction
            objectName: "closeDestinationEditAction"
            icon.name: "window-close"
            text: i18n("Close")
            onTriggered: {
                if (page.dirty) {
                    unsavedDialog.open();
                } else {
                    rootWindow.popSubPage();
                }
            }
        }
    ]

    FolderDialog {
        id: destFolderDialog
        title: i18n("Choose Repository Folder")
        onAccepted: {
            var local = selectedFolder.toString().replace(/^file:\/\//, "");
            if (local.length > 0) {
                page.location = local;
                page.connectionVerified = false;
            }
        }
    }

    Kirigami.PromptDialog {
        id: unsavedDialog
        objectName: "unsavedDestinationDialog"
        title: i18n("Save changes?")
        subtitle: i18n("You have unsaved changes to this destination.")
        standardButtons: Kirigami.Dialog.Save | Kirigami.Dialog.Discard | Kirigami.Dialog.Cancel
        onAccepted: {
            if (page.canSave && page.saveChanges()) {
                rootWindow.popSubPage();
            }
        }
        onDiscarded: {
            appBackend.has_unsaved_changes = false;
            rootWindow.popSubPage();
        }
    }

    Kirigami.FormLayout {
        Kirigami.InlineMessage {
            id: destEditFeedbackBanner
            objectName: "destEditFeedbackBanner"
            Layout.fillWidth: true
            type: testFeedbackOk ? Kirigami.MessageType.Positive : Kirigami.MessageType.Error
            visible: testFeedback !== ""
            text: testFeedback
            actions: [
                Kirigami.Action {
                    objectName: "destEditSwitchToInitAction"
                    icon.name: "folder-new"
                    text: i18n("Create New Repository")
                    visible: !page.testFeedbackOk && page.setupMode === "connect" && appBackend.connection_test_error_kind === "repository_missing"
                    onTriggered: {
                        page.setupMode = "init";
                        page.connectionVerified = false;
                        page.runConnectionTest();
                    }
                }
            ]
        }

        Controls.TextField {
            id: destNameField
            objectName: "destEditNameField"
            Kirigami.FormData.label: i18n("Name:")
            text: page.destName
            onTextChanged: page.destName = text
            Accessible.name: i18n("Destination name")
        }

        Controls.Label {
            visible: !page.nameValid
            text: i18n("Name is required.")
            color: Kirigami.Theme.negativeTextColor
        }

        RowLayout {
            Kirigami.FormData.label: page.kindIsRest ? i18n("Address:") : i18n("Path:")
            Layout.fillWidth: true

            Controls.TextField {
                id: destLocationField
                objectName: "destEditLocationField"
                text: page.location
                onTextChanged: {
                    page.location = text;
                    page.connectionVerified = false;
                }
                placeholderText: page.kindIsRest ? "https://backup.example.org:8000/" : "/mnt/backups"
                Accessible.name: page.kindIsRest ? i18n("REST server address") : i18n("Repository path")
                Layout.fillWidth: true
            }

            Controls.Button {
                visible: !page.kindIsRest
                text: i18n("Choose…")
                onClicked: destFolderDialog.open()
            }
        }

        Controls.Label {
            id: destLocationError
            objectName: "destEditLocationError"
            visible: !page.locationValid
            text: page.kindIsRest
                ? i18n("Enter a valid http:// or https:// address.")
                : i18n("Path is required.")
            color: Kirigami.Theme.negativeTextColor
        }

        Controls.ComboBox {
            id: destEditSetupModeCombo
            objectName: "destEditSetupModeCombo"
            Kirigami.FormData.label: i18n("Repository:")
            model: [i18n("Connect existing repository"), i18n("Create new repository")]
            currentIndex: page.setupMode === "init" ? 1 : 0
            Accessible.name: i18n("Repository setup mode")
            onActivated: {
                page.setupMode = (currentIndex === 1) ? "init" : "connect";
                page.connectionVerified = false;
            }
        }

        Controls.TextField {
            visible: page.kindIsRest
            Kirigami.FormData.label: i18n("Username:")
            text: page.username
            onTextChanged: {
                page.username = text;
                page.connectionVerified = false;
            }
            Accessible.name: i18n("REST username")
        }

        Controls.TextField {
            visible: page.kindIsRest
            Kirigami.FormData.label: i18n("Server password:")
            echoMode: TextInput.Password
            placeholderText: i18n("Leave empty to keep stored password")
            text: page.restPassword
            onTextChanged: {
                page.restPassword = text;
                page.connectionVerified = false;
            }
            Accessible.name: i18n("REST server password")
        }

        Controls.TextField {
            Kirigami.FormData.label: i18n("Repository password:")
            echoMode: TextInput.Password
            placeholderText: i18n("Leave empty to keep stored password")
            text: page.repoPassword
            onTextChanged: {
                page.repoPassword = text;
                page.connectionVerified = false;
            }
            Accessible.name: i18n("Repository password")
        }

        Controls.Button {
            id: testConnButton
            objectName: "destEditTestButton"
            icon.name: "network-connect"
            text: i18n("Test Connection")
            enabled: page.fieldsValid && appBackend.blocking_kind === ""
            Accessible.name: i18n("Test Connection")
            onClicked: page.runConnectionTest()
        }
    }
}
