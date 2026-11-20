// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ScrollablePage {
    id: page
    objectName: "destinationEditPage"
    title: i18n("Edit Connection")

    required property var rootWindow
    required property var appBackend
    required property string destinationId

    property var initialData: null
    // Kept as page properties for callers and tests that read them directly.
    property alias destName: destDraft.name
    property alias setupMode: destDraft.setupMode

    DestinationDraft {
        id: destDraft
        appBackend: page.appBackend
        destinationId: page.destinationId
        setupMode: "connect"
    }

    function loadInitialData() {
        var data = rootWindow.findDestinationData(destinationId);
        initialData = data;
        if (data) {
            destDraft.kind = data.kindName;
            destDraft.name = data.name;
            destDraft.location = data.location;
            destDraft.username = data.username;
        }
    }

    Component.onCompleted: loadInitialData()

    readonly property bool locationChanged: initialData !== null && (
        destDraft.location.trim() !== initialData.location
        || destDraft.username.trim() !== initialData.username
        || destDraft.repoPassword.length > 0
        || destDraft.restPassword.length > 0
    )
    readonly property bool needsVerification: locationChanged || destDraft.setupMode === "init"
    readonly property bool dirty: initialData !== null && (
        destDraft.name.trim() !== initialData.name || needsVerification
    )
    readonly property bool canSave: dirty && destDraft.detailsValid && (!needsVerification || destDraft.connectionVerified) && appBackend.blocking_kind === ""

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

    function saveChanges() {
        if (!canSave || !initialData) return false;
        if (appBackend.updateDestination(
                destinationId,
                destDraft.fieldsJson(),
                destDraft.setupMode,
                destDraft.repoPassword,
                destDraft.restPassword,
                page.needsVerification
            )) {
            initialData = rootWindow.findDestinationData(destinationId);
            destDraft.setupMode = "connect";
            destDraft.repoPassword = "";
            destDraft.restPassword = "";
            destDraft.connectionVerified = false;
            destDraft.testFeedback = "";
            destDraft.testFeedbackOk = false;
            rootWindow.showPassiveNotification(Texts.label("destination_saved"));
            return true;
        }
        destDraft.showError(appBackend.last_action_error);
        return false;
    }

    actions: [
        Kirigami.Action {
            id: saveDestinationAction
            objectName: "saveDestinationAction"
            icon.name: "document-save"
            text: Texts.label("save")
            enabled: page.canSave
            onTriggered: page.saveChanges()
        },
        Kirigami.Action {
            id: closeDestinationEditAction
            objectName: "closeDestinationEditAction"
            icon.name: "window-close"
            text: Texts.label("close")
            onTriggered: {
                if (page.dirty) {
                    unsavedDialog.open();
                } else {
                    rootWindow.popSubPage();
                }
            }
        }
    ]

    Kirigami.PromptDialog {
        id: unsavedDialog
        objectName: "unsavedDestinationDialog"
        title: Texts.label("save_changes_question")
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

    DestinationDetailsForm {
        width: parent.width
        draft: destDraft
        namePrefix: "destEdit"
        editing: true
        setupModes: ["connect", "init"]
    }
}
