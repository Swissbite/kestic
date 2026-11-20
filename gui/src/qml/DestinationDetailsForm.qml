// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// Name, location, repository mode, credentials and connection test of a destination.
// Used by both wizards (new destination) and by the edit page (`editing: true`).
ColumnLayout {
    id: form

    required property var draft
    // Prefix of the object names (`<prefix>NameField`, `<prefix>LocationField`, …).
    required property string namePrefix
    // Edit page: no required-field stars, empty passwords keep the saved ones.
    property bool editing: false
    // Order of the repository modes in the combo box.
    property var setupModes: ["init", "connect"]

    readonly property var appBackend: draft.appBackend

    spacing: Kirigami.Units.mediumSpacing

    function locationEdited() {
        form.draft.connectionVerified = false;
    }

    FolderDialog {
        id: folderDialog
        title: i18n("Choose Repository Folder")
        onAccepted: {
            var local = selectedFolder.toString().replace(/^file:\/\//, "");
            if (local.length > 0) {
                form.draft.location = local;
                form.locationEdited();
            }
        }
    }

    Kirigami.Heading {
        visible: !form.editing
        level: 2
        text: form.draft.kindIsRest ? i18n("Where is the REST server located?") : i18n("Where should the local repository be stored?")
    }

    Kirigami.InlineMessage {
        objectName: form.namePrefix + "FeedbackBanner"
        Layout.fillWidth: true
        type: form.draft.testFeedbackOk ? Kirigami.MessageType.Positive : Kirigami.MessageType.Error
        visible: form.draft.testFeedback !== ""
        text: form.draft.testFeedback
        actions: [
            Kirigami.Action {
                objectName: form.namePrefix + "SwitchToInitAction"
                icon.name: "folder-new"
                text: i18n("Create New Repository")
                visible: form.editing && !form.draft.testFeedbackOk && form.draft.setupMode === "connect"
                    && form.appBackend.connection_test_error_kind === "repository_missing"
                onTriggered: {
                    form.draft.setupMode = "init";
                    form.locationEdited();
                    form.draft.runConnectionTest();
                }
            }
        ]
    }

    Kirigami.FormLayout {
        Layout.fillWidth: true

        Controls.TextField {
            objectName: form.namePrefix + "NameField"
            Kirigami.FormData.label: form.editing ? Texts.label("name_label") : Texts.label("required_name_label")
            text: form.draft.name
            placeholderText: form.editing ? "" : (form.draft.kindIsRest ? i18n("NAS") : i18n("Local Backup"))
            onTextChanged: form.draft.name = text
            Accessible.name: i18n("Destination name")
        }

        Controls.Label {
            objectName: form.namePrefix + "NameError"
            visible: !form.draft.nameValid
            text: i18n("Name is required.")
            color: Kirigami.Theme.negativeTextColor
        }

        RowLayout {
            Kirigami.FormData.label: form.draft.kindIsRest
                ? (form.editing ? i18n("Address:") : i18n("Address*:"))
                : (form.editing ? i18n("Path:") : i18n("Path*:"))
            Layout.fillWidth: true

            Controls.TextField {
                objectName: form.namePrefix + "LocationField"
                text: form.draft.location
                onTextChanged: {
                    form.draft.location = text;
                    form.locationEdited();
                }
                placeholderText: form.draft.kindIsRest ? "https://backup.example.org:8000/" : "/mnt/backups"
                Accessible.name: form.draft.kindIsRest ? i18n("REST server address") : i18n("Repository path")
                Layout.fillWidth: true
            }

            Controls.Button {
                visible: !form.draft.kindIsRest
                text: Texts.label("choose")
                Accessible.name: i18n("Choose repository folder")
                onClicked: folderDialog.open()
            }
        }

        Controls.Label {
            objectName: form.namePrefix + "LocationError"
            visible: !form.draft.locationValid
            text: form.draft.kindIsRest
                ? i18n("Enter a valid http:// or https:// address.")
                : i18n("Path is required.")
            color: Kirigami.Theme.negativeTextColor
        }

        Controls.ComboBox {
            objectName: form.namePrefix + "SetupModeCombo"
            Kirigami.FormData.label: i18n("Repository:")
            model: form.setupModes.map(mode => mode === "init" ? i18n("Create new repository") : i18n("Connect existing repository"))
            currentIndex: Math.max(0, form.setupModes.indexOf(form.draft.setupMode))
            Accessible.name: i18n("Repository setup mode")
            onActivated: {
                form.draft.setupMode = form.setupModes[currentIndex];
                form.locationEdited();
            }
        }

        Controls.TextField {
            visible: form.draft.kindIsRest
            Kirigami.FormData.label: Texts.label("username_label")
            text: form.draft.username
            onTextChanged: {
                form.draft.username = text;
                form.locationEdited();
            }
            Accessible.name: i18n("REST username")
        }

        Controls.TextField {
            visible: form.draft.kindIsRest
            Kirigami.FormData.label: i18n("Server password:")
            echoMode: TextInput.Password
            placeholderText: form.editing ? i18n("Leave empty to keep the saved password") : ""
            text: form.draft.restPassword
            onTextChanged: {
                form.draft.restPassword = text;
                form.locationEdited();
            }
            Accessible.name: i18n("REST server password")
        }

        Controls.TextField {
            visible: form.editing || form.draft.setupMode === "connect"
            Kirigami.FormData.label: form.editing ? i18n("Repository password:") : i18n("Repository password*:")
            echoMode: TextInput.Password
            placeholderText: form.editing ? i18n("Leave empty to keep the saved password") : ""
            text: form.draft.repoPassword
            onTextChanged: {
                if (visible && form.draft.repoPassword !== text) {
                    form.draft.repoPassword = text;
                    form.locationEdited();
                }
            }
            Accessible.name: form.editing ? Texts.label("repository_password") : i18n("Existing repository password")
        }

        Controls.Button {
            objectName: form.namePrefix + "TestButton"
            icon.name: "network-connect"
            text: i18n("Test Connection")
            enabled: form.draft.detailsValid && form.appBackend.blocking_kind === ""
            Accessible.name: text
            onClicked: form.draft.runConnectionTest()
        }
    }
}
