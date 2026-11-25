// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ScrollablePage {
    id: page
    objectName: "historyPage"
    title: Texts.label("history")

    required property var rootWindow
    required property var appBackend
    required property var backupsModel
    required property var runsModel

    property string selectedBackupId: ""
    property string selectedRunId: ""

    // The list whose selected row gets the focus when the page is shown again.
    readonly property var primaryList: runsListView

    function applyFilter() {
        runsModel.setFilter(selectedBackupId, issuesChip.checked, historySearchField.text);
    }

    function buildBackupFilterOptions() {
        var opts = [{ backupId: "", label: Texts.label("all_backups") }];
        var count = backupsModel.rowCount(backupsModel.index(-1, -1));
        for (var i = 0; i < count; ++i) {
            var bid = backupsModel.data(backupsModel.index(i, 0), 0);
            var name = backupsModel.data(backupsModel.index(i, 0), 1);
            if (bid) {
                opts.push({ backupId: bid, label: name || bid });
            }
        }
        backupFilterCombo.model = opts;
        var restoreIdx = 0;
        for (var j = 0; j < opts.length; ++j) {
            if (opts[j].backupId === selectedBackupId) {
                restoreIdx = j;
                break;
            }
        }
        backupFilterCombo.currentIndex = restoreIdx;
    }

    Connections {
        target: appBackend
        function onModelsUpdated() {
            page.buildBackupFilterOptions();
            page.applyFilter();
        }
    }

    Component.onCompleted: {
        buildBackupFilterOptions();
        applyFilter();
    }

    header: Controls.ToolBar {
        contentItem: RowLayout {
            spacing: Kirigami.Units.smallSpacing

            Controls.ComboBox {
                id: backupFilterCombo
                objectName: "historyBackupFilter"
                textRole: "label"
                Accessible.name: i18n("Filter history by backup")
                onActivated: {
                    var item = model[currentIndex];
                    page.selectedBackupId = item ? item.backupId : "";
                    page.applyFilter();
                }
            }

            Kirigami.Chip {
                id: issuesChip
                objectName: "historyIssuesChip"
                text: i18n("Issues")
                checkable: true
                closable: false
                Accessible.name: i18n("Filter runs with issues")
                onToggled: {
                    page.applyFilter();
                    issuesChip.forceActiveFocus(Qt.TabFocusReason);
                }

                FocusRing {
                    target: issuesChip
                    radius: Math.round(issuesChip.height / 2)
                    ringColor: issuesChip.checked ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.highlightColor
                }
            }

            Kirigami.SearchField {
                id: historySearchField
                objectName: "historySearchField"
                Layout.fillWidth: true
                placeholderText: i18n("Search history…")
                Accessible.name: i18n("Search history")
                onTextChanged: page.applyFilter()
            }
        }
    }

    KeyboardList {
        id: runsListView
        objectName: "runsListView"
        model: runsModel
        spacing: Kirigami.Units.smallSpacing
        keyNavigationEnabled: true

        onActiveFocusChanged: {
            if (activeFocus && count > 0 && currentIndex < 0) {
                currentIndex = Math.max(0, runsModel.rowOf(page.selectedRunId));
            }
        }

        Kirigami.PlaceholderMessage {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 4
            visible: runsListView.count === 0
            icon.name: "view-history"
            text: i18n("No matching runs found")
            explanation: i18n("Completed, failed, and active backup runs appear here.")
        }

        section.property: "dayGroup"
        section.delegate: Kirigami.ListSectionHeader {
            required property string section
            width: ListView.view.width
            text: section
        }

        delegate: Controls.ItemDelegate {
            id: runRow
            objectName: "runListItem"
            required property int index
            required property string runId
            required property string title
            required property string statusText
            required property string statusIcon
            required property string statusColorKind
            required property string timeShort
            required property string durationText
            required property string summaryText

            width: ListView.view.width
            focusPolicy: Qt.ClickFocus
            highlighted: page.selectedRunId === runId
            Accessible.name: title
            // Exposed for the smoke test: selected rows must use the selection text colour.
            readonly property color selectedTextColor: Kirigami.Theme.highlightedTextColor

            onClicked: {
                ListView.view.currentIndex = index;
                page.selectedRunId = runId;
                runRow.forceActiveFocus(Qt.TabFocusReason);
                rootWindow.openRunDetailById(runId);
            }

            FocusRing {
                target: runRow
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.mediumSpacing

                Kirigami.Icon {
                    source: runRow.statusIcon
                    color: rootWindow.iconColorFor(runRow.statusColorKind, page.Kirigami.Theme)
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2

                    Controls.Label {
                        objectName: "runRowTitle"
                        text: Texts.labelArgs("two_parts", [runRow.timeShort, runRow.title])
                        font.bold: true
                        color: runRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }

                    Controls.Label {
                        text: runRow.summaryText !== ""
                            ? Texts.labelArgs("two_parts", [runRow.statusText, runRow.summaryText])
                            : Texts.labelArgs("two_parts", [runRow.statusText, runRow.durationText])
                        opacity: 0.8
                        color: runRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }
                }

                Kirigami.Icon {
                    source: "go-next"
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                }
            }
        }
    }
}
