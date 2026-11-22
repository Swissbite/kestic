// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

EditorPage {
    id: page
    objectName: "destinationEditPage"
    title: i18n("Edit Connection")

    required property string destinationId

    property var initialData: null
    // Read by the offscreen smoke.
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
    dirty: initialData !== null && (
        destDraft.name.trim() !== initialData.name || needsVerification
    )
    readonly property bool canSave: dirty && destDraft.detailsValid && (!needsVerification || destDraft.connectionAccepted) && appBackend.blocking_kind === ""

    function saveForGuard() {
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
            onTriggered: page.saveForGuard()
        },
        Kirigami.Action {
            id: closeDestinationEditAction
            objectName: "closeDestinationEditAction"
            icon.name: "window-close"
            text: Texts.label("close")
            onTriggered: page.requestClose()
        }
    ]

    unsavedObjectName: "unsavedDestinationDialog"
    unsavedSubtitle: i18n("You have unsaved changes to this destination.")

    onUnsavedSaveRequested: {
        if (page.canSave && page.saveForGuard()) {
            rootWindow.popSubPage();
        }
    }
    onUnsavedDiscardRequested: {
        rootWindow.popSubPage();
    }

    DestinationDetailsForm {
        width: parent.width
        draft: destDraft
        namePrefix: "destEdit"
        editing: true
        setupModes: ["connect", "init"]
    }
}
