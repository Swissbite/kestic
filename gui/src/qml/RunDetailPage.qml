// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ScrollablePage {
    id: page
    objectName: "runDetailPage"
    title: appBackend.selected_run_title !== "" ? appBackend.selected_run_title : i18n("Run Details")

    required property var rootWindow
    required property var appBackend
    required property var runLogsModel
    required property string runId

    property bool rawErrorExpanded: false
    property string pendingExcludePath: ""
    property string pendingExcludeRuleLabel: ""
    property bool pendingExcludeIsDir: false

    readonly property var unreadableFiles: rootWindow.parseJsonOr(appBackend.selected_run_unreadable_json, [])
    readonly property var commandsList: rootWindow.parseJsonOr(appBackend.selected_run_commands_json, [])

    function refreshDetail() {
        appBackend.loadRunDetail(runId);
        runLogsModel.setFilter(logLevelFilter(), logSearchField.text);
    }

    function logLevelFilter() {
        switch (logFilterCombo.currentIndex) {
        case 1: return "warnings";
        case 2: return "errors";
        default: return "";
        }
    }

    function collectLogText() {
        var lines = [];
        var count = runLogsModel.rowCount(runLogsModel.index(-1, -1));
        for (var i = 0; i < count; ++i) {
            var line = runLogsModel.data(runLogsModel.index(i, 0), 0);
            if (line !== undefined && line !== null) {
                lines.push(line);
            }
        }
        return lines.join("\n");
    }

    function handleEscape() {
        if (addExcludeDialog.opened || addExcludeDialog.visible) {
            addExcludeDialog.close();
            return true;
        }
        return false;
    }

    Component.onCompleted: refreshDetail()

    Connections {
        target: appBackend
        function onModelsUpdated() {
            runLogsModel.sync();
        }
    }

    // Show Command is ALWAYS present in ⋮ regardless of expert_mode
    actions: [
        Kirigami.Action {
            id: runDetailShowCommandAction
            objectName: "runDetailShowCommandAction"
            icon.name: "utilities-terminal"
            text: Texts.label("show_command")
            displayHint: Kirigami.DisplayHint.AlwaysHide
            onTriggered: rootWindow.openShowCommandDialog(
                appBackend.selected_run_transparent_command,
                appBackend.selected_run_title
            )
        }
    ]

    Kirigami.PromptDialog {
        id: addExcludeDialog
        objectName: "addExcludePromptDialog"
        title: i18n("Add to Exclude Rules?")
        subtitle: i18n("New rule (%1): %2\n\nFrom the next backup on, this path is no longer backed up. Existing snapshots do not change.",
                       page.pendingExcludeIsDir ? i18n("folder rule") : i18n("expert file rule"),
                       page.pendingExcludeRuleLabel !== "" ? page.pendingExcludeRuleLabel : page.pendingExcludePath)
        standardButtons: Kirigami.Dialog.Cancel
        customFooterActions: [
            Kirigami.Action {
                id: confirmAddExcludeAction
                objectName: "confirmAddExcludeAction"
                icon.name: "list-add"
                text: i18n("Add and Save")
                onTriggered: {
                    if (appBackend.addExcludePathToBackup(appBackend.selected_run_backup_id, page.pendingExcludePath)) {
                        addExcludeDialog.close();
                        page.refreshDetail();
                    }
                }
            }
        ]
    }

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.largeSpacing

        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            Kirigami.Icon {
                source: appBackend.selected_run_status_icon !== "" ? appBackend.selected_run_status_icon : "state-information"
                Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2

                Controls.Label {
                    id: runStatusLineLabel
                    objectName: "runStatusLineLabel"
                    text: appBackend.selected_run_status_line
                    font.bold: true
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }

                Controls.Label {
                    text: appBackend.selected_run_subtitle
                    opacity: 0.8
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }
            }
        }

        Controls.TabBar {
            id: detailTabBar
            objectName: "runDetailTabBar"
            Layout.fillWidth: true

            Keys.onLeftPressed: function(event) {
                if (currentIndex > 0) {
                    currentIndex -= 1;
                    if (currentItem) currentItem.forceActiveFocus(Qt.TabFocusReason);
                    event.accepted = true;
                }
            }

            Keys.onRightPressed: function(event) {
                if (currentIndex < count - 1) {
                    currentIndex += 1;
                    if (currentItem) currentItem.forceActiveFocus(Qt.TabFocusReason);
                    event.accepted = true;
                }
            }

            Controls.TabButton {
                id: resultTabButton
                text: i18n("Result")
                Accessible.name: i18n("Result")

                FocusRing {
                    target: resultTabButton
                }
            }
            Controls.TabButton {
                id: logTabButton
                text: i18n("Log")
                Accessible.name: i18n("Log")

                FocusRing {
                    target: logTabButton
                }
            }
            Controls.TabButton {
                id: commandsTabButton
                text: i18n("Commands")
                Accessible.name: i18n("Commands")

                FocusRing {
                    target: commandsTabButton
                }
            }
        }

        StackLayout {
            Layout.fillWidth: true
            currentIndex: detailTabBar.currentIndex

            // Tab 0: Result
            ColumnLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing

                Controls.Label {
                    text: appBackend.selected_run_stats_summary
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }

                Kirigami.InlineMessage {
                    Layout.fillWidth: true
                    type: Kirigami.MessageType.Error
                    visible: appBackend.selected_run_error_summary !== ""
                    text: appBackend.selected_run_error_summary
                    actions: [
                        Kirigami.Action {
                            visible: appBackend.selected_run_error_action !== ""
                            text: appBackend.recoveryActionLabel(appBackend.selected_run_error_action)
                            onTriggered: {
                                if (appBackend.selected_run_error_action === "add_to_excludes" && appBackend.selected_run_backup_id !== "") {
                                    rootWindow.openBackupExcludesPage(appBackend.selected_run_backup_id);
                                } else if (appBackend.selected_run_error_action === "check_restic_source") {
                                    rootWindow.openSettingsWindow();
                                } else {
                                    detailTabBar.currentIndex = 1;
                                }
                            }
                        },
                        Kirigami.Action {
                            visible: appBackend.selected_run_error_raw !== ""
                            text: page.rawErrorExpanded ? i18n("Hide Details") : i18n("Details")
                            onTriggered: page.rawErrorExpanded = !page.rawErrorExpanded
                        }
                    ]
                }

                TabNavTextArea {
                    id: rawErrorArea
                    visible: page.rawErrorExpanded && appBackend.selected_run_error_raw !== ""
                    readOnly: true
                    font: Kirigami.Theme.fixedWidthFont
                    text: appBackend.selected_run_error_raw
                    wrapMode: TextEdit.Wrap
                    Layout.fillWidth: true

                    FocusRing {
                        target: rawErrorArea
                    }
                }

                Kirigami.Heading {
                    level: 4
                    visible: page.unreadableFiles.length > 0
                    text: i18n("Unreadable files")
                }

                Repeater {
                    model: page.unreadableFiles

                    delegate: RowLayout {
                        required property string modelData
                        Layout.fillWidth: true
                        spacing: Kirigami.Units.smallSpacing

                        Controls.Label {
                            text: modelData
                            font: Kirigami.Theme.fixedWidthFont
                            elide: Text.ElideMiddle
                            Layout.fillWidth: true
                        }

                        Controls.Button {
                            objectName: "addUnreadableExcludeButton"
                            text: appBackend.recoveryActionLabel("add_to_excludes")
                            visible: appBackend.selected_run_backup_id !== ""
                            Accessible.name: i18n("Add %1 to exclude rules", modelData)
                            onClicked: {
                                page.pendingExcludePath = modelData;
                                page.pendingExcludeRuleLabel = modelData;
                                page.pendingExcludeIsDir = false;
                                var dto = rootWindow.parseJsonOr(appBackend.formatExcludeRuleForItem(modelData), null);
                                if (dto) {
                                    page.pendingExcludeRuleLabel = dto.rule || modelData;
                                    page.pendingExcludeIsDir = Boolean(dto.is_dir);
                                }
                                addExcludeDialog.open();
                            }
                        }
                    }
                }
            }

            // Tab 1: Log
            ColumnLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.smallSpacing

                    Kirigami.SearchField {
                        id: logSearchField
                        objectName: "runLogSearchField"
                        Layout.fillWidth: true
                        placeholderText: i18n("Filter log lines…")
                        Accessible.name: i18n("Filter log lines")
                        onTextChanged: runLogsModel.setFilter(page.logLevelFilter(), text)
                    }

                    Controls.ComboBox {
                        id: logFilterCombo
                        objectName: "runLogFilterCombo"
                        model: [i18n("All"), i18n("Warnings"), i18n("Errors")]
                        Accessible.name: i18n("Filter log severity")
                        onActivated: runLogsModel.setFilter(page.logLevelFilter(), logSearchField.text)
                    }

                    Controls.Button {
                        icon.name: "edit-copy"
                        text: i18n("Copy Log")
                        Accessible.name: i18n("Copy Log")
                        onClicked: {
                            appBackend.copyToClipboard(page.collectLogText());
                            rootWindow.showPassiveNotification(i18n("Log copied to clipboard"));
                        }
                    }

                    Controls.Button {
                        icon.name: "document-open"
                        text: i18n("Open Log File")
                        enabled: appBackend.selected_run_log_path !== ""
                        Accessible.name: i18n("Open Log File")
                        onClicked: Qt.openUrlExternally("file://" + appBackend.selected_run_log_path)
                    }
                }

                Kirigami.PlaceholderMessage {
                    Layout.fillWidth: true
                    visible: runLogsListView.count === 0
                    icon.name: "text-x-generic"
                    text: i18n("No log available for this run")
                }

                ListView {
                    id: runLogsListView
                    objectName: "runLogsListView"
                    Layout.fillWidth: true
                    Layout.preferredHeight: Kirigami.Units.gridUnit * 16
                    visible: count > 0
                    activeFocusOnTab: visible && count > 0
                    keyNavigationEnabled: true
                    clip: true
                    model: runLogsModel

                    onActiveFocusChanged: {
                        if (activeFocus && count > 0 && currentIndex < 0) {
                            currentIndex = 0;
                        }
                    }

                    delegate: Controls.ItemDelegate {
                        id: logRow
                        required property int index
                        required property string lineText
                        required property string level

                        width: ListView.view.width
                        leftPadding: Kirigami.Units.smallSpacing
                        rightPadding: Kirigami.Units.smallSpacing
                        topPadding: 2
                        bottomPadding: 2
                        highlighted: ListView.isCurrentItem && runLogsListView.activeFocus
                        Accessible.name: lineText
                        onClicked: ListView.view.currentIndex = index

                        FocusRing {
                            target: logRow
                            focused: (logRow.activeFocus || logRow.visualFocus || (logRow.ListView.isCurrentItem && runLogsListView.activeFocus))
                            ringColor: logRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.highlightColor
                        }

                        contentItem: Controls.Label {
                            text: logRow.lineText
                            font: Kirigami.Theme.fixedWidthFont
                            wrapMode: Text.WrapAnywhere
                            color: logRow.highlighted
                                ? Kirigami.Theme.highlightedTextColor
                                : (logRow.level === "error"
                                    ? Kirigami.Theme.negativeTextColor
                                    : (logRow.level === "warning" ? Kirigami.Theme.neutralTextColor : Kirigami.Theme.textColor))
                        }
                    }
                }
            }

            // Tab 2: Commands
            ColumnLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing

                Kirigami.PlaceholderMessage {
                    Layout.fillWidth: true
                    visible: page.commandsList.length === 0
                    icon.name: "utilities-terminal"
                    text: i18n("No commands recorded for this run")
                }

                Repeater {
                    model: page.commandsList

                    delegate: Kirigami.AbstractCard {
                        required property var modelData
                        Layout.fillWidth: true

                        contentItem: ColumnLayout {
                            spacing: Kirigami.Units.smallSpacing

                            TabNavTextArea {
                                id: runCmdTextArea
                                objectName: "runCommandTextArea"
                                readOnly: true
                                font: Kirigami.Theme.fixedWidthFont
                                text: modelData.command || ""
                                wrapMode: TextEdit.Wrap
                                Layout.fillWidth: true

                                FocusRing {
                                    target: runCmdTextArea
                                }
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                Controls.Label {
                                    text: modelData.meta || ""
                                    opacity: 0.75
                                    Layout.fillWidth: true
                                }
                                Controls.Button {
                                    icon.name: "edit-copy"
                                    text: Texts.label("copy")
                                    Accessible.name: i18n("Copy command")
                                    onClicked: {
                                        appBackend.copyToClipboard(modelData.command || "");
                                        rootWindow.showPassiveNotification(Texts.label("command_copied"));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
