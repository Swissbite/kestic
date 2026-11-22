// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

WizardPage {
    id: page
    objectName: "addDestinationWizard"
    title: i18n("Add Destination")
    hasInput: destDraft.hasAnyInput
    canGoBack: step > 1
    discardTitle: i18n("Discard new destination?")
    discardObjectName: "discardDestDialog"
    onBackClicked: step -= 1

    required property var appBackend
    property var destinationCreatedCallback: null

    property int step: 1
    // `setupMode` is set by `Main.qml`, `destName` by the offscreen smoke.
    property alias destName: destDraft.name
    property alias setupMode: destDraft.setupMode

    readonly property int totalSteps: destDraft.setupMode === "connect" ? 2 : 3
    readonly property bool isFinalStep: step === totalSteps
    readonly property bool canAdvance: {
        if (step === 1) return typeChooser.choiceValid;
        if (step === 2) return destDraft.detailsValid && destDraft.connectionVerified && (destDraft.setupMode !== "connect" || destDraft.repoPassword.length > 0);
        return destDraft.detailsValid && destDraft.passwordsMatch && appBackend.blocking_kind === "";
    }

    DestinationDraft {
        id: destDraft
        appBackend: page.appBackend
    }

    function finishWizard() {
        if (!canAdvance) return;
        var createdId = appBackend.addDestination(
            destDraft.fieldsJson(),
            destDraft.setupMode,
            destDraft.repoPassword,
            destDraft.restPassword
        );
        if (createdId !== "") {
            rootWindow.showPassiveNotification(Texts.label("destination_saved"));
            rootWindow.closeWizardLayer();
            if (typeof destinationCreatedCallback === "function") {
                destinationCreatedCallback(createdId, destDraft.name.trim());
            }
        } else {
            destDraft.showError(appBackend.last_action_error);
        }
    }

    ColumnLayout {
        width: parent.width

        ColumnLayout {
            Layout.fillWidth: true
            Layout.maximumWidth: Kirigami.Units.gridUnit * 44
            Layout.alignment: Qt.AlignHCenter
            spacing: Kirigami.Units.largeSpacing

            WizardProgress {
                step: page.step
                totalSteps: page.totalSteps
            }

            DestinationTypeChooser {
                id: typeChooser
                visible: page.step === 1
                Layout.fillWidth: true
                draft: destDraft
                namePrefix: "wizDest"
            }

            DestinationDetailsForm {
                visible: page.step === 2
                Layout.fillWidth: true
                draft: destDraft
                namePrefix: "wizDest"
            }

            DestinationPasswordForm {
                visible: page.step === 3
                Layout.fillWidth: true
                draft: destDraft
                namePrefix: "wizDest"
                showErrors: true
            }
        }
    }

    footerButtons: [
        Controls.Button {
            objectName: "wizDestNextButton"
            text: page.isFinalStep ? i18n("Save Destination") : Texts.label("next")
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
    ]
}
