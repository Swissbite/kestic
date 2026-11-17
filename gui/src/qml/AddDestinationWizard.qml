// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.ScrollablePage {
    id: page
    objectName: "addDestinationWizard"
    title: i18n("Add Destination")

    required property var rootWindow
    required property var appBackend
    property var destinationCreatedCallback: null

    property int step: 1
    property string destKind: "local"
    property string setupMode: "init"
    property string destName: ""
    property string location: ""
    property string username: ""
    property string restPassword: ""
    property string repoPassword: ""
    property string repoPasswordConfirm: ""
    property bool connectionVerified: false
    property string testFeedback: ""
    property bool testFeedbackOk: false

    readonly property bool kindIsRest: destKind === "rest"
    readonly property bool hasAnyInput: destName.trim().length > 0 || location.trim().length > 0 || repoPassword.length > 0
    readonly property bool nameValid: destName.trim().length > 0
    readonly property bool locationValid: kindIsRest
        ? /^https?:\/\//i.test(location.trim())
        : location.trim().length > 0
    readonly property bool detailsValid: nameValid && locationValid
    readonly property bool passwordsMatch: repoPassword.length > 0 && (setupMode === "connect" || repoPassword === repoPasswordConfirm)
    readonly property int totalSteps: setupMode === "connect" ? 2 : 3
    readonly property bool isFinalStep: step === totalSteps
    readonly property bool canAdvance: {
        if (step === 1) return true;
        if (step === 2) return detailsValid && connectionVerified && (setupMode !== "connect" || repoPassword.length > 0);
        return detailsValid && passwordsMatch && appBackend.blocking_kind === "";
    }

    onBackRequested: function(event) {
        if (hasAnyInput) {
            event.accepted = true;
            discardDialog.open();
        }
    }

    function handleEscape() {
        if (discardDialog.opened || discardDialog.visible) {
            discardDialog.close();
            return true;
        }
        if (hasAnyInput) {
            discardDialog.open();
            return true;
        }
        return false;
    }

    function runConnectionTest() {
        if (!detailsValid) {
            testFeedbackOk = false;
            testFeedback = i18n("Please fill in all required fields.");
            return false;
        }
        var fieldsJson = JSON.stringify({
            kind: destKind,
            name: destName.trim(),
            location: location.trim(),
            username: username.trim()
        });
        var ok = appBackend.testDestination(
            fieldsJson,
            setupMode,
            repoPassword,
            restPassword
        );
        connectionVerified = ok;
        testFeedbackOk = ok;
        testFeedback = appBackend.connection_test_message;
        return ok;
    }

    function finishWizard() {
        if (!canAdvance) return;
        var fieldsJson = JSON.stringify({
            kind: destKind,
            name: destName.trim(),
            location: location.trim(),
            username: username.trim()
        });
        var createdId = appBackend.addDestination(
            fieldsJson,
            setupMode,
            repoPassword,
            restPassword
        );
        if (createdId !== "") {
            rootWindow.showPassiveNotification(i18n("Destination saved"));
            rootWindow.closeWizardLayer();
            if (typeof destinationCreatedCallback === "function") {
                destinationCreatedCallback(createdId, destName.trim());
            }
        } else {
            testFeedbackOk = false;
            testFeedback = appBackend.last_action_error;
        }
    }

    FolderDialog {
        id: wizardDestFolderDialog
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
        id: discardDialog
        objectName: "discardDestDialog"
        title: i18n("Discard new destination?")
        subtitle: i18n("No repository or configuration has been created yet.")
        standardButtons: Kirigami.Dialog.Discard | Kirigami.Dialog.Cancel
        onDiscarded: rootWindow.closeWizardLayer()
    }

    ColumnLayout {
        width: parent.width

        ColumnLayout {
            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 44
            Layout.alignment: Qt.AlignHCenter
            spacing: Kirigami.Units.largeSpacing

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing

                Controls.Label {
                    text: i18n("Step %1 of %2", page.step, page.totalSteps)
                    font.bold: true
                }

                Controls.ProgressBar {
                    from: 1
                    to: page.totalSteps
                    value: page.step
                    Layout.fillWidth: true
                }
            }

        // Step 1: Type
        ColumnLayout {
            visible: page.step === 1
            Layout.fillWidth: true
            spacing: Kirigami.Units.mediumSpacing

            Kirigami.Heading {
                level: 2
                text: i18n("Where should encrypted backups be stored?")
            }

            Controls.RadioButton {
                id: localTypeRadio
                objectName: "destTypeLocalRadio"
                text: i18n("Local Folder or External Drive")
                checked: page.destKind === "local"
                Accessible.name: text
                onToggled: if (checked) {
                    page.destKind = "local";
                    page.connectionVerified = false;
                }
            }

            Controls.Label {
                text: i18n("Store snapshots in a local directory or mounted drive path.")
                opacity: 0.75
                Layout.leftMargin: Kirigami.Units.largeSpacing
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
            }

            Controls.RadioButton {
                id: restTypeRadio
                objectName: "destTypeRestRadio"
                text: i18n("REST Server")
                checked: page.destKind === "rest"
                Accessible.name: text
                onToggled: if (checked) {
                    page.destKind = "rest";
                    page.connectionVerified = false;
                }
            }

            Controls.Label {
                text: i18n("Connect to a restic REST server over HTTPS or HTTP.")
                opacity: 0.75
                Layout.leftMargin: Kirigami.Units.largeSpacing
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
            }
        }

        // Step 2: Details & Connection Test
        ColumnLayout {
            visible: page.step === 2
            Layout.fillWidth: true
            spacing: Kirigami.Units.mediumSpacing

            Kirigami.Heading {
                level: 2
                text: page.kindIsRest ? i18n("Where is the REST server located?") : i18n("Where should the local repository be stored?")
            }

            Kirigami.InlineMessage {
                Layout.fillWidth: true
                type: page.testFeedbackOk ? Kirigami.MessageType.Positive : Kirigami.MessageType.Error
                visible: page.testFeedback !== ""
                text: page.testFeedback
            }

            Kirigami.FormLayout {
                Layout.fillWidth: true

                Controls.TextField {
                    id: wizNameField
                    objectName: "wizDestNameField"
                    Kirigami.FormData.label: i18n("Name*:")
                    text: page.destName
                    placeholderText: page.kindIsRest ? i18n("NAS") : i18n("Local Backup")
                    onTextChanged: page.destName = text
                    Accessible.name: i18n("Destination name")
                }

                Controls.Label {
                    id: wizNameError
                    objectName: "wizDestNameError"
                    visible: !page.nameValid
                    text: i18n("Name is required.")
                    color: Kirigami.Theme.negativeTextColor
                }

                RowLayout {
                    Kirigami.FormData.label: page.kindIsRest ? i18n("Address*:") : i18n("Path*:")
                    Layout.fillWidth: true

                    Controls.TextField {
                        id: wizLocField
                        objectName: "wizDestLocationField"
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
                        Accessible.name: i18n("Choose repository folder")
                        onClicked: wizardDestFolderDialog.open()
                    }
                }

                Controls.Label {
                    id: wizLocError
                    objectName: "wizDestLocationError"
                    visible: !page.locationValid
                    text: page.kindIsRest
                        ? i18n("Enter a valid http:// or https:// address.")
                        : i18n("Path is required.")
                    color: Kirigami.Theme.negativeTextColor
                }

                Controls.ComboBox {
                    Kirigami.FormData.label: i18n("Repository:")
                    model: [i18n("Create new repository"), i18n("Connect existing repository")]
                    currentIndex: page.setupMode === "connect" ? 1 : 0
                    Accessible.name: i18n("Repository setup mode")
                    onActivated: {
                        page.setupMode = (currentIndex === 1) ? "connect" : "init";
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
                    text: page.restPassword
                    onTextChanged: {
                        page.restPassword = text;
                        page.connectionVerified = false;
                    }
                    Accessible.name: i18n("REST server password")
                }

                Controls.TextField {
                    visible: page.setupMode === "connect"
                    Kirigami.FormData.label: i18n("Repository password*:")
                    echoMode: TextInput.Password
                    text: page.repoPassword
                    onTextChanged: {
                        if (visible && page.setupMode === "connect" && page.repoPassword !== text) {
                            page.repoPassword = text;
                            page.connectionVerified = false;
                        }
                    }
                    Accessible.name: i18n("Existing repository password")
                }

                Controls.Button {
                    id: wizTestConnBtn
                    objectName: "wizDestTestButton"
                    icon.name: "network-connect"
                    text: i18n("Test Connection")
                    enabled: page.detailsValid && appBackend.blocking_kind === ""
                    Accessible.name: i18n("Test Connection")
                    onClicked: page.runConnectionTest()
                }
            }
        }

        // Step 3: Password
        ColumnLayout {
            visible: page.step === 3
            Layout.fillWidth: true
            spacing: Kirigami.Units.mediumSpacing

            Kirigami.Heading {
                level: 2
                text: i18n("Backup Encryption Password")
            }

            Controls.Label {
                text: i18n("Your backups are encrypted with this password. Kestic stores it securely in your password store.")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }

            Kirigami.InlineMessage {
                Layout.fillWidth: true
                type: Kirigami.MessageType.Error
                visible: page.testFeedback !== "" && !page.testFeedbackOk
                text: page.testFeedback
            }

            Kirigami.FormLayout {
                Layout.fillWidth: true

                Controls.TextField {
                    id: wizRepoPwField
                    objectName: "wizDestPasswordField"
                    Kirigami.FormData.label: i18n("Backup password*:")
                    echoMode: TextInput.Password
                    text: page.repoPassword
                    onTextChanged: page.repoPassword = text
                    Accessible.name: i18n("Backup password")
                }

                Controls.TextField {
                    id: wizRepoPwConfirmField
                    objectName: "wizDestPasswordConfirmField"
                    visible: page.setupMode === "init"
                    Kirigami.FormData.label: i18n("Repeat password*:")
                    echoMode: TextInput.Password
                    text: page.repoPasswordConfirm
                    onTextChanged: page.repoPasswordConfirm = text
                    Accessible.name: i18n("Repeat backup password")
                }

                Controls.Label {
                    visible: page.setupMode === "init" && page.repoPasswordConfirm.length > 0 && page.repoPassword !== page.repoPasswordConfirm
                    text: i18n("Passwords do not match.")
                    color: Kirigami.Theme.negativeTextColor
                }
            }
        }
        }
    }

    footer: Controls.ToolBar {
        contentItem: RowLayout {
            spacing: Kirigami.Units.smallSpacing

            Controls.Button {
                text: i18n("Cancel")
                Accessible.name: i18n("Cancel")
                onClicked: {
                    if (page.hasAnyInput) {
                        discardDialog.open();
                    } else {
                        rootWindow.closeWizardLayer();
                    }
                }
            }

            Item { Layout.fillWidth: true }

            Controls.Button {
                text: i18n("Back")
                visible: page.step > 1
                Accessible.name: i18n("Back")
                onClicked: page.step -= 1
            }

            Controls.Button {
                id: wizNextBtn
                objectName: "wizDestNextButton"
                text: page.isFinalStep ? i18n("Save Destination") : i18n("Next")
                highlighted: true
                enabled: page.canAdvance
                Accessible.name: text
                onClicked: {
                    if (!page.isFinalStep) {
                        page.step += 1;
                    } else {
                        page.finishWizard();
                    }
                }
            }
        }
    }
}
