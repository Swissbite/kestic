// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ApplicationWindow {
    id: settingsWindow
    objectName: "settingsWindow"
    title: i18n("Configure Kestic")
    width: 720
    height: 640
    minimumWidth: 520
    minimumHeight: 460
    visible: false

    required property var appBackend

    property string pendingProviderKind: "automatic"
    property string pendingCustomPath: ""

    // Applies the choice of the source dialog and closes it when the daemon accepted it or the
    // download of the restic managed by Kestic started.
    function applyResticSourceChoice() {
        if (settingsWindow.pendingProviderKind === "custom" && settingsWindow.pendingCustomPath.trim().length === 0) {
            return;
        }
        if (appBackend.applyResticSource(settingsWindow.pendingProviderKind, settingsWindow.pendingCustomPath)) {
            resticSourceDialog.close();
        }
    }

    function saveSettingsFromUi() {
        appBackend.updateSettings(
            defaultOverdueSpinBox.value,
            notifErrorSwitch.checked,
            notifWarningSwitch.checked,
            notifOverdueSwitch.checked,
            notifMaintenanceSwitch.checked,
            notifResticUpdateSwitch.checked
        );
    }

    onClosing: function(close) {
        close.accepted = false;
        visible = false;
    }

    // Opens the window; with `section` "restic" it scrolls to the restic section.
    function openWindow(section) {
        visible = true;
        if (Qt.platform.pluginName !== "offscreen") {
            raise();
        }
        requestActivate();
        if (section === "restic") {
            resticScrollTimer.restart();
        }
    }

    // The restic section is the last one: scrolling to the end shows it.
    function showResticSection() {
        var page = pageStack.initialPage;
        var flick = page ? page.flickable : null;
        if (!flick) return;
        flick.contentY = Math.max(0, flick.contentHeight - flick.height);
    }

    // Opens the restic section with the focus on the download offered there; the download
    // starts only with a click on it.
    function showResticDownload() {
        openWindow("restic");
        if (downloadResticButton.visible) {
            downloadResticButton.forceActiveFocus();
        }
    }

    // A window that was hidden knows its page height only after its first layout pass.
    Timer {
        id: resticScrollTimer
        interval: 100
        onTriggered: settingsWindow.showResticSection()
    }

    // Polls the installation while it runs, also while this window is hidden.
    Timer {
        objectName: "resticInstallPollTimer"
        interval: 500
        repeat: true
        running: appBackend.restic_install_state === "running"
        onTriggered: appBackend.pollResticInstall()
    }

    function canActivateFocusedControlWithReturn() {
        var it = settingsWindow.activeFocusItem;
        if (!it) return false;
        if (it.hasOwnProperty("textDocument") || it.hasOwnProperty("acceptableInput")
                || it.hasOwnProperty("popup") || it.hasOwnProperty("highlightedIndex")
                || it.hasOwnProperty("subMenu")) {
            return false;
        }
        if (it.Accessible && (it.Accessible.role === Accessible.MenuItem
                || it.Accessible.role === Accessible.ComboBox
                || it.Accessible.role === Accessible.EditableText)) {
            return false;
        }
        return typeof it.animateClick === "function"
            || (typeof it.clicked === "function" && typeof it.down !== "undefined");
    }

    function activateFocusedControlWithReturn() {
        var it = settingsWindow.activeFocusItem;
        if (!canActivateFocusedControlWithReturn()) return;
        if (it.checkable && typeof it.animateClick === "function") {
            it.animateClick();
        } else if (typeof it.clicked === "function") {
            it.clicked();
        } else if (typeof it.animateClick === "function") {
            it.animateClick();
        }
    }

    function handleEscape() {
        if (resticSourceDialog.opened || resticSourceDialog.visible) {
            resticSourceDialog.close();
            return true;
        }
        settingsWindow.close();
        return true;
    }

    Kirigami.Dialog {
        id: resticSourceDialog
        objectName: "resticSourceDialog"
        title: i18n("Change restic Source")
        standardButtons: Kirigami.Dialog.Cancel
        padding: Kirigami.Units.largeSpacing

        customFooterActions: [
            Kirigami.Action {
                id: applyResticSourceAction
                objectName: "applyResticSourceAction"
                icon.name: "dialog-ok-apply"
                text: i18n("Apply")
                enabled: settingsWindow.pendingProviderKind !== "custom" || settingsWindow.pendingCustomPath.trim().length > 0
                onTriggered: settingsWindow.applyResticSourceChoice()
            }
        ]

        ColumnLayout {
            spacing: Kirigami.Units.mediumSpacing
            implicitWidth: Kirigami.Units.gridUnit * 24

            Controls.ButtonGroup {
                id: resticSourceGroup
            }

            Controls.RadioButton {
                objectName: "resticSourceAutomaticRadio"
                text: Texts.label("restic_automatic")
                checked: settingsWindow.pendingProviderKind === "automatic"
                Controls.ButtonGroup.group: resticSourceGroup
                Accessible.name: text
                Accessible.description: automaticHint.text
                onToggled: if (checked) settingsWindow.pendingProviderKind = "automatic"
            }

            Controls.Label {
                id: automaticHint
                text: i18n("The system restic from 0.18.0 on, otherwise the restic managed by Kestic.")
                opacity: 0.75
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.gridUnit * 2
            }

            Controls.RadioButton {
                objectName: "resticSourceSystemRadio"
                text: Texts.label("restic_system")
                checked: settingsWindow.pendingProviderKind === "system"
                Controls.ButtonGroup.group: resticSourceGroup
                Accessible.name: text
                Accessible.description: systemHint.text
                onToggled: if (checked) settingsWindow.pendingProviderKind = "system"
            }

            Controls.Label {
                id: systemHint
                text: i18n("The restic installed on the system (from PATH).")
                opacity: 0.75
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.gridUnit * 2
            }

            Controls.RadioButton {
                objectName: "resticSourceAppManagedRadio"
                text: Texts.label("restic_app_managed")
                checked: settingsWindow.pendingProviderKind === "app_managed"
                Controls.ButtonGroup.group: resticSourceGroup
                Accessible.name: text
                Accessible.description: appManagedHint.text
                onToggled: if (checked) settingsWindow.pendingProviderKind = "app_managed"
            }

            Controls.Label {
                id: appManagedHint
                objectName: "resticSourceAppManagedHint"
                text: appBackend.restic_app_managed_installed
                    ? i18n("The official restic release that Kestic downloaded and checked.")
                    : i18n("Not installed yet: Apply downloads and checks the official restic release first.")
                opacity: 0.75
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.gridUnit * 2
            }

            Controls.RadioButton {
                objectName: "resticSourceCustomRadio"
                text: Texts.label("restic_custom")
                checked: settingsWindow.pendingProviderKind === "custom"
                Controls.ButtonGroup.group: resticSourceGroup
                Accessible.name: text
                onToggled: if (checked) settingsWindow.pendingProviderKind = "custom"
            }

            Controls.TextField {
                id: customResticPathField
                objectName: "customResticPathField"
                enabled: settingsWindow.pendingProviderKind === "custom"
                text: settingsWindow.pendingCustomPath
                onTextChanged: settingsWindow.pendingCustomPath = text
                onAccepted: settingsWindow.applyResticSourceChoice()
                placeholderText: "/usr/local/bin/restic"
                Accessible.name: i18n("Custom restic program path")
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.gridUnit * 2
            }

            Controls.Label {
                visible: appBackend.last_action_error !== ""
                text: appBackend.last_action_error
                textFormat: Text.PlainText
                color: Kirigami.Theme.negativeTextColor
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
        }
    }

    pageStack.initialPage: Kirigami.ScrollablePage {
        title: i18n("Configure Kestic")

        Kirigami.FormLayout {
            Kirigami.Separator {
                Kirigami.FormData.isSection: true
                Kirigami.FormData.label: i18n("Appearance")
            }

            Controls.Switch {
                id: expertModeSwitch
                objectName: "expertModeSwitch"
                Kirigami.FormData.label: i18n("Expert mode:")
                checked: appBackend.expert_mode
                Accessible.name: i18n("Expert mode")
                onToggled: appBackend.updateExpertMode(checked)
            }

            Controls.Label {
                text: i18n("Shows technical identifiers, snapshot tags, and expert options.")
                opacity: 0.75
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            Kirigami.Separator {
                Kirigami.FormData.isSection: true
                Kirigami.FormData.label: i18n("Monitoring & Notifications")
            }

            Controls.SpinBox {
                id: defaultOverdueSpinBox
                objectName: "defaultOverdueSpinBox"
                // The digits are centred in the whole box: Fusion makes the box lower than the
                // padding of the style allows and would cut them.
                topPadding: 0
                bottomPadding: 0
                Kirigami.FormData.label: i18n("Mark overdue after missed runs:")
                from: 1
                to: 99
                value: appBackend.default_overdue_threshold
                Accessible.name: i18n("Mark overdue after missed runs")
                onValueModified: settingsWindow.saveSettingsFromUi()
            }

            Controls.Label {
                text: i18n("A scheduled backup counts as overdue when this many scheduled runs in a row did not happen.")
                opacity: 0.75
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            Controls.Switch {
                id: notifErrorSwitch
                objectName: "notifErrorSwitch"
                Kirigami.FormData.label: i18n("Notify on backup errors:")
                checked: appBackend.notif_on_error
                Accessible.name: i18n("Notify on backup errors")
                onToggled: settingsWindow.saveSettingsFromUi()
            }

            Controls.Switch {
                id: notifWarningSwitch
                objectName: "notifWarningSwitch"
                Kirigami.FormData.label: i18n("Notify on warnings:")
                checked: appBackend.notif_on_warning
                Accessible.name: i18n("Notify on warnings")
                onToggled: settingsWindow.saveSettingsFromUi()
            }

            Controls.Switch {
                id: notifOverdueSwitch
                objectName: "notifOverdueSwitch"
                Kirigami.FormData.label: i18n("Notify when backup is overdue:")
                checked: appBackend.notif_on_overdue
                Accessible.name: i18n("Notify when backup is overdue")
                onToggled: settingsWindow.saveSettingsFromUi()
            }

            Controls.Switch {
                id: notifMaintenanceSwitch
                objectName: "notifMaintenanceSwitch"
                Kirigami.FormData.label: i18n("Notify on maintenance failures:")
                checked: appBackend.notif_on_maintenance_failed
                Accessible.name: i18n("Notify on maintenance failures")
                onToggled: settingsWindow.saveSettingsFromUi()
            }

            Controls.Switch {
                id: notifResticUpdateSwitch
                objectName: "notifResticUpdateSwitch"
                Kirigami.FormData.label: i18n("Notify on restic updates:")
                checked: appBackend.notif_on_restic_update
                Accessible.name: i18n("Notify on restic updates")
                onToggled: settingsWindow.saveSettingsFromUi()
            }

            Kirigami.Separator {
                Kirigami.FormData.isSection: true
                Kirigami.FormData.label: i18n("Background Service")
            }

            Controls.Label {
                Kirigami.FormData.label: Texts.label("status_label")
                text: appBackend.daemon_reachable
                    ? i18n("Running · starts when you log in")
                    : i18n("Not running")
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            Controls.Label {
                text: i18n("Closing the window does not quit Kestic; backups keep running. To quit, use the Kestic icon in the system tray.")
                opacity: 0.75
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            Kirigami.Separator {
                Kirigami.FormData.isSection: true
                Kirigami.FormData.label: i18n("restic")
            }

            Controls.Label {
                objectName: "resticSourceLabel"
                Kirigami.FormData.label: Texts.label("source_label")
                text: appBackend.restic_source_text
                textFormat: Text.PlainText
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            Controls.Label {
                objectName: "resticVersionLabel"
                Kirigami.FormData.label: i18n("Version:")
                visible: appBackend.restic_version_text !== ""
                text: appBackend.restic_version_text
                textFormat: Text.PlainText
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            // A path of the file system: plain text, broken anywhere so a long one stays inside.
            Controls.Label {
                objectName: "resticProgramLabel"
                Kirigami.FormData.label: i18n("Program:")
                visible: appBackend.restic_program_text !== ""
                text: appBackend.restic_program_text
                textFormat: Text.PlainText
                wrapMode: Text.WrapAtWordBoundaryOrAnywhere
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            Controls.Label {
                objectName: "resticAvailableLabel"
                Kirigami.FormData.label: i18n("Available:")
                visible: appBackend.restic_available_text !== ""
                text: appBackend.restic_available_text
                textFormat: Text.PlainText
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            // Why restic cannot be used, with the raw reason under «Details».
            ColumnLayout {
                id: resticProblemPanel
                spacing: Kirigami.Units.smallSpacing
                visible: appBackend.restic_problem_text !== ""
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26

                property bool detailsExpanded: false

                // The reason names the path the user entered: plain text, never markup.
                RowLayout {
                    spacing: Kirigami.Units.smallSpacing
                    Layout.fillWidth: true

                    Kirigami.Icon {
                        source: "dialog-error"
                        implicitWidth: Kirigami.Units.iconSizes.smallMedium
                        implicitHeight: Kirigami.Units.iconSizes.smallMedium
                        Layout.alignment: Qt.AlignTop
                    }

                    Controls.Label {
                        objectName: "resticProblemMessage"
                        text: appBackend.restic_problem_text
                        textFormat: Text.PlainText
                        color: Kirigami.Theme.negativeTextColor
                        wrapMode: Text.WrapAtWordBoundaryOrAnywhere
                        Layout.fillWidth: true
                    }

                    Controls.Button {
                        objectName: "resticProblemDetailsButton"
                        visible: appBackend.restic_problem_detail !== ""
                        flat: true
                        icon.name: resticProblemPanel.detailsExpanded ? "go-up" : "go-down"
                        text: resticProblemPanel.detailsExpanded ? Texts.label("hide_details") : Texts.label("details")
                        Accessible.name: text
                        Layout.alignment: Qt.AlignTop
                        onClicked: resticProblemPanel.detailsExpanded = !resticProblemPanel.detailsExpanded
                    }
                }

                // Raw text of the operating system: shown as plain text, never as markup.
                Controls.Label {
                    objectName: "resticProblemDetail"
                    visible: resticProblemPanel.detailsExpanded && appBackend.restic_problem_detail !== ""
                    text: appBackend.restic_problem_detail
                    textFormat: Text.PlainText
                    font: Kirigami.Theme.fixedWidthFont
                    wrapMode: Text.WrapAtWordBoundaryOrAnywhere
                    Layout.fillWidth: true
                }
            }

            Controls.Label {
                objectName: "resticMinimumLabel"
                visible: appBackend.restic_explain_minimum
                text: i18n("Kestic needs restic 0.18.0 or newer: older versions do not report errors and check results in a form Kestic can read.")
                opacity: 0.75
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            Kirigami.InlineMessage {
                objectName: "resticSwitchMessage"
                type: Kirigami.MessageType.Information
                visible: appBackend.restic_offer_switch
                text: i18n("System restic is now recent enough – switch?")
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
                actions: [
                    Kirigami.Action {
                        objectName: "switchResticToAutomaticAction"
                        text: Texts.label("restic_automatic")
                        enabled: appBackend.daemon_reachable
                        onTriggered: {
                            appBackend.switchResticToAutomatic();
                            changeResticSourceButton.forceActiveFocus();
                        }
                    }
                ]
            }

            Kirigami.InlineMessage {
                objectName: "resticRemoveMessage"
                type: Kirigami.MessageType.Information
                visible: appBackend.restic_offer_remove
                text: i18n("The restic managed by Kestic is no longer used.")
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
                actions: [
                    Kirigami.Action {
                        objectName: "removeAppManagedResticAction"
                        icon.name: "edit-delete"
                        text: i18n("Delete")
                        enabled: appBackend.restic_offer_enabled
                        onTriggered: {
                            appBackend.removeAppManagedRestic();
                            changeResticSourceButton.forceActiveFocus();
                        }
                    }
                ]
            }

            Controls.Label {
                objectName: "resticActionError"
                visible: appBackend.restic_action_error !== ""
                text: appBackend.restic_action_error
                textFormat: Text.PlainText
                color: Kirigami.Theme.negativeTextColor
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }

            RowLayout {
                spacing: Kirigami.Units.smallSpacing

                Controls.Button {
                    id: changeResticSourceButton
                    objectName: "changeResticSourceButton"
                    text: i18n("Change restic Source…")
                    Accessible.name: i18n("Change restic Source")
                    onClicked: {
                        settingsWindow.pendingProviderKind = appBackend.restic_provider_kind !== ""
                            ? appBackend.restic_provider_kind : "automatic";
                        settingsWindow.pendingCustomPath = appBackend.restic_provider_kind === "custom"
                            ? appBackend.restic_binary_path : "";
                        resticSourceDialog.open();
                    }
                }

                Controls.Button {
                    id: downloadResticButton
                    objectName: "downloadResticButton"
                    visible: appBackend.restic_offer_download
                    enabled: appBackend.restic_offer_enabled
                    icon.name: "download"
                    text: i18n("Download restic")
                    Accessible.name: text
                    onClicked: appBackend.startResticInstall()
                }

                Controls.Button {
                    id: updateResticButton
                    objectName: "updateResticButton"
                    visible: appBackend.restic_offer_update
                    enabled: appBackend.restic_offer_enabled
                    icon.name: "system-software-update"
                    text: i18n("Update Now")
                    Accessible.name: i18n("Update restic now")
                    onClicked: appBackend.startResticInstall()
                }
            }

            // Progress, result or error of the last installation of the restic managed by Kestic.
            ColumnLayout {
                id: resticInstallPanel
                objectName: "resticInstallPanel"
                spacing: Kirigami.Units.smallSpacing
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26

                readonly property string installState: appBackend.restic_install_state
                readonly property bool running: installState === "running"
                property bool detailsExpanded: false

                visible: installState === "running" || installState === "done" || installState === "error"

                onInstallStateChanged: detailsExpanded = false

                Controls.Label {
                    objectName: "resticInstallPhaseLabel"
                    visible: resticInstallPanel.running
                    text: appBackend.restic_install_phase_text
                    textFormat: Text.PlainText
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }

                RowLayout {
                    visible: resticInstallPanel.running
                    spacing: Kirigami.Units.smallSpacing
                    Layout.fillWidth: true

                    Controls.ProgressBar {
                        objectName: "resticInstallProgressBar"
                        from: 0
                        to: 1
                        indeterminate: appBackend.restic_install_progress < 0
                        value: Math.max(0, appBackend.restic_install_progress)
                        Accessible.name: appBackend.restic_install_phase_text
                        Layout.fillWidth: true
                    }

                    Controls.Button {
                        objectName: "cancelResticInstallButton"
                        icon.name: "process-stop"
                        text: Texts.label("cancel")
                        enabled: appBackend.restic_install_cancellable
                        Accessible.name: text
                        onClicked: appBackend.cancelResticInstall()
                    }
                }

                Kirigami.InlineMessage {
                    objectName: "resticInstallResultMessage"
                    type: Kirigami.MessageType.Positive
                    visible: resticInstallPanel.installState === "done"
                    text: appBackend.restic_install_result
                    Layout.fillWidth: true
                }

                Kirigami.InlineMessage {
                    objectName: "resticInstallErrorMessage"
                    type: Kirigami.MessageType.Error
                    visible: resticInstallPanel.installState === "error"
                    text: appBackend.restic_install_error
                    Layout.fillWidth: true
                    actions: [
                        Kirigami.Action {
                            objectName: "resticInstallDetailsAction"
                            visible: appBackend.restic_install_error_detail !== ""
                            icon.name: resticInstallPanel.detailsExpanded ? "go-up" : "go-down"
                            text: resticInstallPanel.detailsExpanded ? Texts.label("hide_details") : Texts.label("details")
                            onTriggered: resticInstallPanel.detailsExpanded = !resticInstallPanel.detailsExpanded
                        }
                    ]
                }

                // Raw text from the network or from gpgv: shown as plain text, never as markup.
                Controls.Label {
                    objectName: "resticInstallErrorDetail"
                    visible: resticInstallPanel.installState === "error" && resticInstallPanel.detailsExpanded
                        && appBackend.restic_install_error_detail !== ""
                    text: appBackend.restic_install_error_detail
                    textFormat: Text.PlainText
                    font: Kirigami.Theme.fixedWidthFont
                    wrapMode: Text.WrapAtWordBoundaryOrAnywhere
                    Layout.fillWidth: true
                }
            }
        }
    }
}
