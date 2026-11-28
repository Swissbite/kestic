// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

EditorPage {
    id: page
    revisionOf: "destination"
    objectName: "destinationEditPage"
    title: i18n("Edit Connection")

    required property string destinationId

    property var initialData: null
    // `connectionKey()` of the saved destination.
    property string initialConnection: ""
    // Read by the offscreen smoke.
    property alias setupMode: destDraft.setupMode

    DestinationDraft {
        id: destDraft
        appBackend: page.appBackend
        destinationId: page.destinationId
        setupMode: "connect"
    }

    // Shows the destination as the daemon has it now, with its revision; typed passwords go.
    function loadInitialData() {
        var data = rootWindow.findDestinationData(destinationId);
        initialData = data;
        revision = appBackend.destinationRevision(destinationId);
        if (data) {
            destDraft.loadFromRow(data);
            initialConnection = destDraft.connectionKey();
        }
    }

    Component.onCompleted: loadInitialData()

    readonly property bool locationChanged: initialData !== null && (
        destDraft.connectionKey() !== initialConnection
        || destDraft.repoPassword.length > 0
        || destDraft.restPassword.length > 0
        || destDraft.accessKeyId.length > 0
        || destDraft.secretAccessKey.length > 0
        || destDraft.envList().some(function(v) { return v.secret && v.value.length > 0; })
    )
    readonly property bool needsVerification: locationChanged || destDraft.setupMode === "init"
    dirty: initialData !== null && (
        destDraft.name.trim() !== initialData.name || needsVerification
    )
    readonly property bool canSave: dirty && destDraft.detailsValid && (!needsVerification || destDraft.connectionAccepted) && appBackend.blocking_kind === ""

    // Saves; `done(ok)` runs once the daemon has answered.
    function saveForGuard(done) {
        if (!canSave || !initialData || saving) {
            if (done) done(false);
            return;
        }
        var fields = JSON.parse(destDraft.fieldsJson());
        fields.revision = page.revision;
        var request = appBackend.updateDestination(
                destinationId,
                JSON.stringify(fields),
                destDraft.setupMode,
                destDraft.repoPassword,
                destDraft.backendSecretsJson(),
                page.needsVerification
            );
        saving = true;
        rootWindow.afterDaemonCall(request, function(ok, answer) {
            saving = false;
            page.afterSave(ok, answer, function() {
                page.loadInitialData();
                destDraft.setupMode = "connect";
                destDraft.clearSecrets();
                destDraft.connectionVerified = false;
                destDraft.testFeedback = "";
                destDraft.testFeedbackOk = false;
            });
            if (ok) {
                initialData = rootWindow.findDestinationData(destinationId);
                initialConnection = destDraft.connectionKey();
                destDraft.setupMode = "connect";
                destDraft.clearSecrets();
                destDraft.connectionVerified = false;
                destDraft.testFeedback = "";
                destDraft.testFeedbackOk = false;
                rootWindow.showPassiveNotification(Texts.label("destination_saved"));
            } else if (answer !== "conflict") {
                destDraft.showError(appBackend.last_action_error);
                if (appBackend.connection_test_error_kind === "other_repository") {
                    destDraft.connectionVerified = false;
                }
            }
            if (done) done(ok);
        });
    }

    actions: [
        Kirigami.Action {
            id: saveDestinationAction
            objectName: "saveDestinationAction"
            icon.name: "document-save"
            text: Texts.label("save")
            enabled: page.canSave && !page.saving
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
        if (page.canSave) {
            page.saveForGuard(function(ok) {
                if (ok && rootWindow.pageStack.currentItem === page) {
                    rootWindow.popSubPage();
                }
            });
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
        onAddAsNewRequested: rootWindow.openAddDestinationWizardWith(destDraft.prefillData())
    }
}
