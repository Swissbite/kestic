// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ScrollablePage {
    readonly property SharedTexts sharedTexts: SharedTexts {}
    id: page
    objectName: "newBackupWizard"
    title: wizardTitle

    required property var rootWindow
    required property var appBackend
    required property var destinationsModel

    property string wizardTitle: i18n("New Backup")
    property bool integratedDestFlow: false

    // Linear stage progression:
    // - Existing destinations: source -> choose_dest -> summary (3 steps)
    // - No destinations yet (new repo): source -> dest_type -> dest_details -> dest_password -> summary (5 steps)
    // - No destinations yet (connect existing repo): source -> dest_type -> dest_details -> summary (4 steps)
    readonly property var stages: {
        if (!integratedDestFlow) {
            return ["source", "choose_dest", "summary"];
        }
        if (setupMode === "connect") {
            return ["source", "dest_type", "dest_details", "summary"];
        }
        return ["source", "dest_type", "dest_details", "dest_password", "summary"];
    }
    property int stepIndex: 0
    readonly property int displayStep: stepIndex + 1
    readonly property int totalSteps: stages.length
    readonly property string currentStage: stages[Math.min(stepIndex, stages.length - 1)]

    // Backup state
    property string sourcePath: appBackend.homeDir()
    property string selectedDestId: ""
    property string selectedDestName: ""
    property string backupName: ""
    property string excludesText: ""
    property bool nameManuallyEdited: false
    property string saveErrorText: ""
    property string detectedSourceKind: "folder"
    property string detectedDriveLabel: ""
    property var nestedWarnings: []

    function refreshSourceMetadata() {
        var trimmed = sourcePath.trim();
        if (trimmed === "") {
            detectedSourceKind = "folder";
            detectedDriveLabel = "";
            nestedWarnings = [];
            return;
        }
        var detRaw = appBackend.detectSource(trimmed);
        if (detRaw !== "") {
            try {
                var det = JSON.parse(detRaw);
                detectedSourceKind = det.kind || "folder";
                detectedDriveLabel = det.drive_label || "";
            } catch (e) {
            }
        }
        if (!integratedDestFlow && selectedDestId !== "") {
            var nestRaw = appBackend.checkNestedSources("", trimmed, selectedDestId);
            try {
                nestedWarnings = JSON.parse(nestRaw || "[]");
            } catch (e) {
                nestedWarnings = [];
            }
        } else {
            nestedWarnings = [];
        }
    }

    // Integrated destination creation state (used when integratedDestFlow === true)
    property string destKind: "local"
    property string setupMode: "init"
    property string destName: ""
    property string destLocation: ""
    property string destUsername: ""
    property string restPassword: ""
    property string repoPassword: ""
    property string repoPasswordConfirm: ""
    property bool connectionVerified: false
    property string testFeedback: ""
    property bool testFeedbackOk: false
    property string createdDestId: ""

    readonly property bool kindIsRest: destKind === "rest"
    readonly property bool sourceValid: sourcePath.trim().length > 0
    readonly property bool destValid: selectedDestId.length > 0
    readonly property bool destNameValid: destName.trim().length > 0
    readonly property bool destLocationValid: kindIsRest
        ? /^https?:\/\//i.test(destLocation.trim())
        : destLocation.trim().length > 0
    readonly property bool destDetailsValid: destNameValid && destLocationValid && (setupMode !== "connect" || repoPassword.length > 0)
    readonly property bool passwordsMatch: repoPassword.length > 0 && (setupMode === "connect" || repoPassword === repoPasswordConfirm)
    readonly property bool hasAnyDestInput: integratedDestFlow && (destName.trim().length > 0 || destLocation.trim().length > 0 || repoPassword.length > 0)

    readonly property bool summaryValid: {
        if (backupName.trim().length === 0 || !sourceValid) return false;
        if (integratedDestFlow) {
            return destDetailsValid && connectionVerified && passwordsMatch;
        }
        return destValid;
    }

    readonly property bool canAdvance: {
        switch (currentStage) {
        case "source":
            return sourceValid;
        case "choose_dest":
            return destValid;
        case "dest_type":
            return true;
        case "dest_details":
            return destDetailsValid && connectionVerified;
        case "dest_password":
            return passwordsMatch;
        default:
            return summaryValid && appBackend.blocking_kind === "";
        }
    }

    onBackRequested: function(event) {
        if (hasAnyDestInput) {
            event.accepted = true;
            discardDialog.open();
        }
    }

    function handleEscape() {
        if (discardDialog.opened || discardDialog.visible) {
            discardDialog.close();
            return true;
        }
        if (hasAnyDestInput) {
            discardDialog.open();
            return true;
        }
        return false;
    }

    function deriveDefaultName(path) {
        var trimmed = path.replace(/\/+$/, "");
        var idx = trimmed.lastIndexOf("/");
        var base = idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
        return base.length > 0 ? base : i18n("Personal folder");
    }

    function preselectFirstDestinationIfNeeded() {
        var count = destinationsModel.rowCount(destinationsModel.index(-1, -1));
        if (count > 0 && selectedDestId === "") {
            selectedDestId = destinationsModel.data(destinationsModel.index(0, 0), 0) || "";
            selectedDestName = destinationsModel.data(destinationsModel.index(0, 0), 1) || "";
        }
    }

    function runIntegratedConnectionTest() {
        if (!destNameValid || !destLocationValid) {
            testFeedbackOk = false;
            testFeedback = i18n("Please fill in all required fields.");
            return false;
        }
        var fieldsJson = JSON.stringify({
            kind: destKind,
            name: destName.trim(),
            location: destLocation.trim(),
            username: destUsername.trim()
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

    function submitBackup(startImmediately) {
        if (!summaryValid) return;
        saveErrorText = "";
        var targetDestId = selectedDestId;
        if (integratedDestFlow) {
            if (createdDestId === "") {
                var fieldsJson = JSON.stringify({
                    kind: destKind,
                    name: destName.trim(),
                    location: destLocation.trim(),
                    username: destUsername.trim()
                });
                var newDestId = appBackend.addDestination(
                    fieldsJson,
                    setupMode,
                    repoPassword,
                    restPassword
                );
                if (newDestId === "") {
                    saveErrorText = appBackend.last_action_error;
                    return;
                }
                createdDestId = newDestId;
            }
            targetDestId = createdDestId;
            selectedDestId = createdDestId;
            selectedDestName = destName.trim();
        }

        var createdId = appBackend.addBackup(
            backupName.trim(),
            sourcePath.trim(),
            targetDestId,
            excludesText
        );
        if (createdId !== "") {
            rootWindow.showPassiveNotification(i18n("Backup saved"));
            rootWindow.closeWizardLayer();
            if (startImmediately) {
                appBackend.startBackup(createdId);
            }
        } else {
            saveErrorText = appBackend.last_action_error;
        }
    }

    Component.onCompleted: {
        backupName = deriveDefaultName(sourcePath);
        preselectFirstDestinationIfNeeded();
    }

    Connections {
        target: appBackend
        function onModelsUpdated() {
            page.preselectFirstDestinationIfNeeded();
        }
    }

    FolderDialog {
        id: sourcePickDialog
        title: i18n("Choose Source Folder")
        onAccepted: {
            var local = selectedFolder.toString().replace(/^file:\/\//, "");
            if (local.length > 0) {
                page.sourcePath = local;
                if (!page.nameManuallyEdited) {
                    page.backupName = page.deriveDefaultName(local);
                }
            }
        }
    }

    FolderDialog {
        id: destFolderPickDialog
        title: i18n("Choose Repository Folder")
        onAccepted: {
            var local = selectedFolder.toString().replace(/^file:\/\//, "");
            if (local.length > 0) {
                page.destLocation = local;
                page.connectionVerified = false;
            }
        }
    }

    Kirigami.PromptDialog {
        id: discardDialog
        title: i18n("Discard backup setup?")
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
                    id: wizStepCounterLabel
                    objectName: "wizBackupStepLabel"
                    text: i18n("Step %1 of %2", page.displayStep, page.totalSteps)
                    font.bold: true
                }

                Controls.ProgressBar {
                    from: 1
                    to: page.totalSteps
                    value: page.displayStep
                    Layout.fillWidth: true
                }
            }

            // Stage: Source
            ColumnLayout {
                visible: page.currentStage === "source"
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing

                Kirigami.Heading {
                    level: 2
                    text: i18n("What should be backed up?")
                }

                Kirigami.FormLayout {
                    Layout.fillWidth: true

                    RowLayout {
                        Kirigami.FormData.label: i18n("Source folder*:")
                        Layout.fillWidth: true

                        Controls.TextField {
                            id: wizSourceField
                            objectName: "wizBackupSourceField"
                            text: page.sourcePath
                            onTextChanged: {
                                page.sourcePath = text;
                                if (!page.nameManuallyEdited) {
                                    page.backupName = page.deriveDefaultName(text);
                                }
                                page.refreshSourceMetadata();
                            }
                            Accessible.name: i18n("Source folder path")
                            Layout.fillWidth: true
                        }

                        Controls.Button {
                            text: i18n("Choose Other Folder…")
                            icon.name: "folder-open"
                            Accessible.name: i18n("Choose source folder")
                            onClicked: sourcePickDialog.open()
                        }
                    }
                }

                Kirigami.InlineMessage {
                    id: wizDriveDetectedBanner
                    objectName: "wizDriveDetectedBanner"
                    Layout.fillWidth: true
                    type: Kirigami.MessageType.Information
                    visible: page.detectedSourceKind === "drive"
                    text: page.detectedDriveLabel !== ""
                        ? i18n("External drive '%1' detected — backed up only when plugged in.", page.detectedDriveLabel)
                        : i18n("External drive detected — backed up only when plugged in.")
                }
            }

            // Stage: Choose existing destination (when destinations already exist)
            ColumnLayout {
                visible: page.currentStage === "choose_dest"
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing

                Kirigami.Heading {
                    level: 2
                    text: i18n("Where should the backup be stored?")
                }

                ListView {
                    id: wizDestList
                    objectName: "wizBackupDestList"
                    Layout.fillWidth: true
                    Layout.preferredHeight: Math.max(contentHeight, Kirigami.Units.gridUnit * 8)
                    model: destinationsModel
                    spacing: Kirigami.Units.smallSpacing
                    activeFocusOnTab: visible && count > 0
                    keyNavigationEnabled: true

                    onActiveFocusChanged: {
                        if (activeFocus && count > 0 && currentIndex < 0) {
                            var targetIdx = 0;
                            for (var i = 0; i < count; ++i) {
                                if (destinationsModel.data(destinationsModel.index(i, 0), 0) === page.selectedDestId) {
                                    targetIdx = i;
                                    break;
                                }
                            }
                            currentIndex = targetIdx;
                        }
                    }

                    delegate: Controls.ItemDelegate {
                        id: destRow
                        objectName: "wizBackupDestItem"
                        required property int index
                        required property string destinationId
                        required property string name
                        required property string kindLabel
                        required property string statusText
                        required property string statusIcon

                        width: ListView.view.width
                        highlighted: page.selectedDestId === destinationId
                        Accessible.name: name

                        onClicked: {
                            ListView.view.currentIndex = index;
                            page.selectedDestId = destinationId;
                            page.selectedDestName = name;
                            page.refreshSourceMetadata();
                        }

                        Rectangle {
                            anchors.fill: parent
                            z: 10
                            color: "transparent"
                            radius: Kirigami.Units.cornerRadius
                            border.width: (destRow.activeFocus || destRow.visualFocus) ? 2 : 0
                            border.color: destRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.highlightColor
                            visible: border.width > 0
                        }

                        contentItem: RowLayout {
                            spacing: Kirigami.Units.mediumSpacing
                            Kirigami.Icon {
                                source: destRow.statusIcon
                                Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                                Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                            }
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 2
                                Controls.Label {
                                    text: destRow.name
                                    font.bold: true
                                    Layout.fillWidth: true
                                }
                                Controls.Label {
                                    text: destRow.kindLabel + " · " + destRow.statusText
                                    opacity: 0.75
                                    Layout.fillWidth: true
                                }
                            }
                        }
                    }
                }

                Controls.Button {
                    text: i18n("Add New Destination…")
                    icon.name: "list-add"
                    enabled: appBackend.blocking_kind === ""
                    Accessible.name: i18n("Add New Destination")
                    onClicked: rootWindow.openAddDestinationWizard(function(newId, newName) {
                        page.selectedDestId = newId;
                        page.selectedDestName = newName;
                    })
                }
            }

            // Stage: Integrated Destination Type (when no destinations exist yet)
            ColumnLayout {
                visible: page.currentStage === "dest_type"
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing

                Kirigami.Heading {
                    level: 2
                    text: i18n("Where should encrypted backups be stored?")
                }

                Controls.RadioButton {
                    objectName: "wizBackupDestLocalRadio"
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
                    objectName: "wizBackupDestRestRadio"
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

            // Stage: Integrated Destination Details & Connection Test
            ColumnLayout {
                visible: page.currentStage === "dest_details"
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
                        id: wizBackupDestNameField
                        objectName: "wizBackupDestNameField"
                        Kirigami.FormData.label: i18n("Name*:")
                        text: page.destName
                        placeholderText: page.kindIsRest ? i18n("NAS") : i18n("Local Backup")
                        onTextChanged: page.destName = text
                        Accessible.name: i18n("Destination name")
                    }

                    Controls.Label {
                        visible: !page.destNameValid
                        text: i18n("Name is required.")
                        color: Kirigami.Theme.negativeTextColor
                    }

                    RowLayout {
                        Kirigami.FormData.label: page.kindIsRest ? i18n("Address*:") : i18n("Path*:")
                        Layout.fillWidth: true

                        Controls.TextField {
                            id: wizBackupDestLocField
                            objectName: "wizBackupDestLocationField"
                            text: page.destLocation
                            onTextChanged: {
                                page.destLocation = text;
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
                            onClicked: destFolderPickDialog.open()
                        }
                    }

                    Controls.Label {
                        visible: !page.destLocationValid
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
                        text: page.destUsername
                        onTextChanged: {
                            page.destUsername = text;
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
                        id: wizBackupDestTestBtn
                        objectName: "wizBackupDestTestButton"
                        icon.name: "network-connect"
                        text: i18n("Test Connection")
                        enabled: page.destNameValid && page.destLocationValid && appBackend.blocking_kind === ""
                        Accessible.name: i18n("Test Connection")
                        onClicked: page.runIntegratedConnectionTest()
                    }
                }
            }

            // Stage: Integrated Backup Encryption Password
            ColumnLayout {
                visible: page.currentStage === "dest_password"
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

                Kirigami.FormLayout {
                    Layout.fillWidth: true

                    Controls.TextField {
                        id: wizBackupDestPwField
                        objectName: "wizBackupDestPasswordField"
                        Kirigami.FormData.label: i18n("Backup password*:")
                        echoMode: TextInput.Password
                        text: page.repoPassword
                        onTextChanged: page.repoPassword = text
                        Accessible.name: i18n("Backup password")
                    }

                    Controls.TextField {
                        id: wizBackupDestPwConfirmField
                        objectName: "wizBackupDestPasswordConfirmField"
                        Kirigami.FormData.label: i18n("Repeat password*:")
                        echoMode: TextInput.Password
                        text: page.repoPasswordConfirm
                        onTextChanged: page.repoPasswordConfirm = text
                        Accessible.name: i18n("Repeat backup password")
                    }

                    Controls.Label {
                        visible: page.repoPasswordConfirm.length > 0 && page.repoPassword !== page.repoPasswordConfirm
                        text: i18n("Passwords do not match.")
                        color: Kirigami.Theme.negativeTextColor
                    }
                }
            }

            // Stage: Summary & Excludes
            ColumnLayout {
                visible: page.currentStage === "summary"
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing

                Kirigami.Heading {
                    level: 2
                    text: i18n("Summary")
                }

                Kirigami.InlineMessage {
                    Layout.fillWidth: true
                    type: Kirigami.MessageType.Error
                    visible: page.saveErrorText !== "" || appBackend.last_action_error !== ""
                    text: page.saveErrorText !== "" ? page.saveErrorText : appBackend.last_action_error
                }

                Kirigami.InlineMessage {
                    id: wizNestedSourceWarningBanner
                    objectName: "wizNestedSourceWarningBanner"
                    Layout.fillWidth: true
                    type: Kirigami.MessageType.Warning
                    visible: page.nestedWarnings.length > 0
                    text: page.nestedWarnings.length > 0
                        ? i18n("This source overlaps with backup '%1' (%2) on the same destination. You can still create the backup, but check the exclude rules.",
                               page.nestedWarnings[0].other_backup_name,
                               page.nestedWarnings[0].other_source_path)
                        : ""
                }

                Kirigami.FormLayout {
                    Layout.fillWidth: true

                    Controls.TextField {
                        id: wizBackupNameField
                        objectName: "wizBackupNameField"
                        Kirigami.FormData.label: i18n("Name*:")
                        text: page.backupName
                        onTextEdited: {
                            page.nameManuallyEdited = true;
                            page.backupName = text;
                        }
                        Accessible.name: i18n("Backup name")
                    }

                    Controls.Label {
                        Kirigami.FormData.label: i18n("Source:")
                        text: page.sourcePath
                        wrapMode: Text.WordWrap
                    }

                    Controls.Label {
                        Kirigami.FormData.label: i18n("Destination:")
                        text: page.integratedDestFlow ? page.destName.trim() : page.selectedDestName
                    }

                    Controls.Label {
                        Kirigami.FormData.label: i18n("Defaults:")
                        text: i18n("Daily at 12:00 (AC power & network) · Standard retention (7 daily, 4 weekly, 12 monthly)")
                        wrapMode: Text.WordWrap
                    }

                    Controls.ScrollView {
                        Kirigami.FormData.label: i18n("Exclude rules:")
                        Layout.fillWidth: true
                        Layout.preferredHeight: Kirigami.Units.gridUnit * 7

                        Controls.TextArea {
                            id: wizBackupExcludesArea
                            objectName: "wizBackupExcludesArea"
                            font: Kirigami.Theme.fixedWidthFont
                            text: page.excludesText
                            onTextChanged: page.excludesText = text
                            placeholderText: "*.tmp\n.cache"
                            Accessible.name: i18n("Exclude rules in restic syntax")

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

                            Rectangle {
                                anchors.fill: parent
                                z: 10
                                color: "transparent"
                                radius: Kirigami.Units.cornerRadius
                                border.width: wizBackupExcludesArea.activeFocus ? 2 : 0
                                border.color: Kirigami.Theme.highlightColor
                                visible: border.width > 0
                            }
                        }
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
                    if (page.hasAnyDestInput) {
                        discardDialog.open();
                    } else {
                        rootWindow.closeWizardLayer();
                    }
                }
            }

            Item { Layout.fillWidth: true }

            Controls.Button {
                text: i18n("Back")
                visible: page.stepIndex > 0
                Accessible.name: i18n("Back")
                onClicked: page.stepIndex -= 1
            }

            Controls.Button {
                id: wizBackupNextBtn
                objectName: "wizBackupNextButton"
                visible: page.currentStage !== "summary"
                text: i18n("Next")
                highlighted: true
                enabled: page.canAdvance
                Accessible.name: i18n("Next")
                onClicked: page.stepIndex += 1
            }

            Controls.Button {
                id: saveLaterBtn
                objectName: "wizBackupSaveLaterButton"
                visible: page.currentStage === "summary"
                text: i18n("Save, Back Up Later")
                enabled: page.canAdvance
                Accessible.name: i18n("Save, Back Up Later")
                onClicked: page.submitBackup(false)
            }

            Controls.Button {
                id: backUpNowBtn
                objectName: "wizBackupStartNowButton"
                visible: page.currentStage === "summary"
                icon.name: "media-playback-start"
                text: sharedTexts.label("back_up_now")
                highlighted: true
                enabled: page.canAdvance
                Accessible.name: i18n("Save and Back Up Now")
                onClicked: page.submitBackup(true)
            }
        }
    }
}
