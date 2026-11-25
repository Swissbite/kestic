// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// Restore area, right column: the snapshots of one backup, newest first, grouped by day, or
// those of one further group (unassigned, orphaned, not configured here) per destination.
// Return opens a snapshot for browsing; the context menu (Menu key, Shift+F10 or right click)
// also compares it with the previous one or restores it. The search field (Ctrl+F) above the
// list searches all snapshots of the backup. «Compare…» turns the list into a choice of exactly
// two snapshots (Return or Space marks one) whose differences «Show Differences» opens.
Kirigami.ScrollablePage {
    id: page
    objectName: "restoreSnapshotsPage"
    title: backupName

    required property var rootWindow
    required property var appBackend
    required property var restoreSnapshotsModel
    required property string backupId
    property string backupName: ""
    // The further group listed instead of a backup; empty for a backup.
    property string group: ""
    // Why destinations of the group could not be listed.
    property string groupError: ""

    readonly property bool loading: group === ""
        ? appBackend.restore_snapshots_loading : appBackend.restore_overview_loading
    readonly property string errorText: group === "" ? appBackend.restore_snapshots_error : groupError

    // The list whose selected row gets the focus when the page is shown again.
    readonly property var primaryList: restoreSnapshotsListView

    signal browseRequested(var destinationId, var snapshotId)
    signal compareRequested(var destinationId, var fromSnapshotId, var toSnapshotId)
    signal restoreRequested(var destinationId, var snapshotId)
    signal searchRequested(var backupId, var text)

    // The list chooses two snapshots to compare instead of opening one.
    property bool compareMode: false
    // The snapshots chosen to compare, in the order they were chosen.
    property var compareIds: []
    // The destination of the chosen snapshots; both must be on the same one.
    property string compareDestinationId: ""

    function syncRows() {
        restoreSnapshotsModel.sync();
        groupError = restoreSnapshotsModel.groupError();
    }

    function reload() {
        endCompare();
        restoreSnapshotsListView.currentIndex = -1;
        restoreSnapshotsModel.setGroup(group);
        if (group === "") {
            appBackend.loadRestoreSnapshots(backupId);
        } else {
            syncRows();
        }
    }

    // Lists the snapshots of backup `newBackupId`, or of the further group `newGroup`, under
    // the title `title`.
    function showSource(newBackupId, newGroup, title) {
        backupName = title;
        group = newGroup;
        backupId = newBackupId;
        searchField.text = "";
        reload();
    }

    function startCompare() {
        compareIds = [];
        compareDestinationId = "";
        compareMode = true;
        restoreSnapshotsListView.forceActiveFocus(Qt.TabFocusReason);
    }

    function endCompare() {
        compareMode = false;
        compareIds = [];
        compareDestinationId = "";
    }

    // Whether the snapshot in `row` can be chosen next to those chosen already.
    function canChoose(row) {
        return compareIds.length < 2
            && (compareDestinationId === "" || compareDestinationId === row.destinationId);
    }

    // Chooses the snapshot in `row` to compare, or takes it back.
    function toggleCompare(row) {
        if (compareIds.indexOf(row.snapshotId) >= 0) {
            var rest = compareIds.filter(function(id) { return id !== row.snapshotId; });
            compareIds = rest;
            if (rest.length === 0) compareDestinationId = "";
        } else if (canChoose(row)) {
            compareIds = compareIds.concat([row.snapshotId]);
            compareDestinationId = row.destinationId;
        }
    }

    // Compares the two chosen snapshots, from the older to the newer (the list is newest
    // first).
    function showDifferences() {
        if (compareIds.length !== 2) return;
        var a = restoreSnapshotsModel.rowOf(compareIds[0]);
        var b = restoreSnapshotsModel.rowOf(compareIds[1]);
        var older = a > b ? compareIds[0] : compareIds[1];
        var newer = a > b ? compareIds[1] : compareIds[0];
        var destinationId = compareDestinationId;
        endCompare();
        page.compareRequested(destinationId, older, newer);
    }

    // Opens the context menu of snapshot `row`: at the point `x`, `y` of the row (a click), or
    // below the row without a point (the keyboard).
    function openSnapshotMenu(row, x, y) {
        restoreSnapshotsListView.currentIndex = row.index;
        snapshotMenu.destinationId = row.destinationId;
        snapshotMenu.snapshotId = row.snapshotId;
        snapshotMenu.previousSnapshotId = row.previousSnapshotId;
        if (x === undefined) {
            snapshotMenu.popup(row, 0, row.height);
        } else {
            snapshotMenu.popup(row, x, y);
        }
    }

    Component.onCompleted: reload()

    Connections {
        target: appBackend
        function onRestoreSnapshotsUpdated() {
            if (page.group === "") restoreSnapshotsModel.sync();
        }
        function onRestoreOverviewUpdated() {
            if (page.group !== "") page.syncRows();
        }
    }

    actions: [
        Kirigami.Action {
            objectName: "restoreCompareAction"
            icon.name: "vcs-diff"
            text: i18n("Compare…")
            enabled: !page.compareMode && restoreSnapshotsListView.count >= 2
            onTriggered: page.startCompare()
        },
        Kirigami.Action {
            objectName: "restoreSnapshotsRefreshAction"
            icon.name: "view-refresh"
            text: Texts.label("refresh")
            enabled: !page.loading
            onTriggered: {
                // The list of backups beside it refreshes too: this is the only «Refresh» then.
                if (page.group === "") {
                    page.reload();
                    appBackend.loadRestoreOverview();
                } else {
                    page.endCompare();
                    appBackend.loadRestoreOverview();
                }
            }
        }
    ]

    // The search field stays in the page's header while the snapshots scroll below it; the
    // header is as high as that of the search results beside it.
    header: Controls.ToolBar {
        objectName: "snapshotSearchHeader"
        visible: page.group === ""
        leftPadding: Kirigami.Units.largeSpacing
        rightPadding: Kirigami.Units.largeSpacing
        contentItem: RowLayout {
            Kirigami.SearchField {
                id: searchField
                objectName: "snapshotSearchField"
                Layout.fillWidth: true
                placeholderText: Texts.label("search_in_backup_snapshots")
                Accessible.name: Texts.label("search_in_backup_snapshots")
                // Ctrl+F (the field's default) focuses this field; the other search and filter
                // fields of the restore area have none, so the shortcut is never ambiguous.
                // Searching runs restic over every snapshot, so it starts on Return only.
                autoAccept: false
                onAccepted: {
                    if (text.trim() !== "") page.searchRequested(page.backupId, text);
                }
            }
        }
    }

    footer: Controls.ToolBar {
        visible: page.compareMode
        contentItem: RowLayout {
            spacing: Kirigami.Units.mediumSpacing
            Controls.Label {
                objectName: "compareChosenLabel"
                Layout.fillWidth: true
                text: i18n("%1 of 2 chosen", page.compareIds.length)
                elide: Text.ElideRight
            }
            Controls.Button {
                objectName: "showDifferencesButton"
                icon.name: "vcs-diff"
                text: i18n("Show Differences")
                enabled: page.compareIds.length === 2
                onClicked: page.showDifferences()
            }
            Controls.Button {
                objectName: "cancelCompareButton"
                text: Texts.label("cancel")
                onClicked: page.endCompare()
            }
        }
    }

    ColumnLayout {
        width: parent.width
        spacing: Kirigami.Units.mediumSpacing

        Kirigami.LoadingPlaceholder {
            Layout.fillWidth: true
            visible: page.loading
            text: Texts.label("loading_snapshots")
        }

        Kirigami.InlineMessage {
            objectName: "restoreSnapshotsError"
            Layout.fillWidth: true
            type: Kirigami.MessageType.Error
            visible: !page.loading && page.errorText !== ""
            text: page.errorText
            actions: [
                Kirigami.Action {
                    text: Texts.label("retry")
                    onTriggered: page.group === "" ? page.reload() : appBackend.loadRestoreOverview()
                }
            ]
        }

        Kirigami.PlaceholderMessage {
            objectName: "restoreSnapshotsEmpty"
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.gridUnit * 2
            visible: !page.loading && restoreSnapshotsListView.count === 0 && page.errorText === ""
            icon.name: "view-history"
            text: page.group === "unassigned" ? i18n("No unassigned snapshots")
                : page.group === "orphaned" ? i18n("No orphaned backups")
                : Texts.label("no_snapshots_yet")
        }

        KeyboardList {
            id: restoreSnapshotsListView
            objectName: "restoreSnapshotsListView"
            Layout.fillWidth: true
            Layout.preferredHeight: contentHeight
            visible: !page.loading && count > 0
            interactive: false
            keyNavigationEnabled: true
            model: restoreSnapshotsModel
            spacing: Kirigami.Units.smallSpacing

            onActiveFocusChanged: {
                if (activeFocus && count > 0 && currentIndex < 0) {
                    currentIndex = 0;
                }
            }

            section.property: "sectionName"
            // A section name may carry details after its first line (a backup not configured
            // here: number of snapshots, newest one); they wrap below the heading.
            section.delegate: ColumnLayout {
                id: sectionItem
                required property string section
                readonly property int lineBreak: section.indexOf("\n")
                width: ListView.view.width
                spacing: 0

                Kirigami.ListSectionHeader {
                    id: sectionHeader
                    objectName: "restoreSnapshotSectionHeader"
                    Layout.fillWidth: true
                    text: sectionItem.lineBreak < 0 ? sectionItem.section
                        : sectionItem.section.substring(0, sectionItem.lineBreak)
                }
                Controls.Label {
                    objectName: "restoreSnapshotSectionDetail"
                    visible: sectionItem.lineBreak >= 0
                    text: visible ? sectionItem.section.substring(sectionItem.lineBreak + 1) : ""
                    wrapMode: Text.Wrap
                    opacity: 0.75
                    leftPadding: sectionHeader.leftPadding
                    rightPadding: sectionHeader.rightPadding
                    Layout.fillWidth: true
                }
            }

            delegate: Controls.ItemDelegate {
                id: snapRow
                objectName: "restoreSnapshotListItem"
                required property int index
                required property string snapshotId
                required property string shortId
                required property string title
                required property string subtitle
                required property string previousSnapshotId
                required property string destinationId
                required property string hint
                required property string tags

                readonly property bool chosen: page.compareIds.indexOf(snapshotId) >= 0

                width: ListView.view.width
                focusPolicy: Qt.ClickFocus
                highlighted: ListView.isCurrentItem
                Accessible.name: title
                Accessible.description: page.compareMode && chosen
                    ? i18n("Chosen to compare")
                    : (hint === "" ? subtitle : Texts.labelArgs("two_parts", [subtitle, hint]))

                onClicked: {
                    ListView.view.currentIndex = index;
                    if (page.compareMode) {
                        page.toggleCompare(snapRow);
                    } else {
                        page.browseRequested(destinationId, snapshotId);
                    }
                }

                Keys.onPressed: function(event) {
                    if (page.compareMode && event.key === Qt.Key_Space) {
                        page.toggleCompare(snapRow);
                        event.accepted = true;
                    } else if (page.compareMode && event.key === Qt.Key_Escape) {
                        page.endCompare();
                        event.accepted = true;
                    } else if (event.key === Qt.Key_Menu
                            || (event.key === Qt.Key_F10 && (event.modifiers & Qt.ShiftModifier))) {
                        page.openSnapshotMenu(snapRow);
                        event.accepted = true;
                    }
                }

                // The menu opens on the press of the right button, at the pointer, like KDE's
                // context menus; a hold or a small move before the release changes nothing.
                MouseArea {
                    anchors.fill: parent
                    acceptedButtons: Qt.RightButton
                    onPressed: function(mouse) {
                        page.openSnapshotMenu(snapRow, mouse.x, mouse.y);
                    }
                }

                FocusRing {
                    target: snapRow
                }

                contentItem: RowLayout {
                    spacing: Kirigami.Units.mediumSpacing

                    Controls.CheckBox {
                        objectName: "compareSnapshotCheck"
                        visible: page.compareMode
                        checked: snapRow.chosen
                        enabled: snapRow.chosen || page.canChoose(snapRow)
                        focusPolicy: Qt.NoFocus
                        Accessible.name: i18n("Choose %1 to compare", snapRow.title)
                        onToggled: page.toggleCompare(snapRow)
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2

                        RowLayout {
                            Layout.fillWidth: true
                            Controls.Label {
                                text: snapRow.title
                                color: snapRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                                Layout.fillWidth: true
                            }
                            Controls.Label {
                                visible: appBackend.expert_mode
                                text: snapRow.shortId
                                font: Kirigami.Theme.fixedWidthFont
                                color: snapRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                                opacity: 0.7
                            }
                        }
                        Controls.Label {
                            objectName: "restoreSnapshotSubtitle"
                            text: snapRow.subtitle
                            color: snapRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                            opacity: 0.75
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                        Controls.Label {
                            objectName: "restoreSnapshotTags"
                            visible: appBackend.expert_mode && snapRow.tags !== ""
                            text: snapRow.tags
                            font: Kirigami.Theme.fixedWidthFont
                            color: snapRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                            opacity: 0.7
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                        RowLayout {
                            visible: snapRow.hint !== ""
                            spacing: Kirigami.Units.smallSpacing
                            Layout.fillWidth: true
                            RowIcon {
                                source: "dialog-information"
                                onSelection: snapRow.highlighted
                            }
                            Controls.Label {
                                objectName: "restoreSnapshotHint"
                                text: snapRow.hint
                                color: snapRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                                opacity: 0.75
                                elide: Text.ElideRight
                                Layout.fillWidth: true
                            }
                        }
                    }

                    Kirigami.Icon {
                        visible: !page.compareMode
                        source: "go-next-symbolic"
                        selected: snapRow.highlighted
                        Layout.preferredWidth: Kirigami.Units.iconSizes.small
                        Layout.preferredHeight: Kirigami.Units.iconSizes.small
                    }
                }
            }
        }
    }

    Controls.Menu {
        id: snapshotMenu
        objectName: "snapshotContextMenu"

        property string destinationId: ""
        property string snapshotId: ""
        property string previousSnapshotId: ""

        Controls.MenuItem {
            objectName: "browseSnapshotMenuItem"
            icon.name: "document-open-folder"
            text: i18n("Browse")
            onTriggered: page.browseRequested(snapshotMenu.destinationId, snapshotMenu.snapshotId)
        }
        Controls.MenuItem {
            objectName: "compareWithPreviousMenuItem"
            icon.name: "kdiff3"
            text: i18n("Compare with Previous")
            enabled: snapshotMenu.previousSnapshotId !== ""
            onTriggered: page.compareRequested(snapshotMenu.destinationId,
                snapshotMenu.previousSnapshotId, snapshotMenu.snapshotId)
        }
        Controls.MenuItem {
            objectName: "restoreSnapshotMenuItem"
            icon.name: "document-revert"
            text: i18n("Restore…")
            onTriggered: page.restoreRequested(snapshotMenu.destinationId, snapshotMenu.snapshotId)
        }

        onClosed: {
            var item = restoreSnapshotsListView.itemAtIndex(restoreSnapshotsListView.currentIndex);
            if (item) item.forceActiveFocus(Qt.TabFocusReason);
        }
    }
}
