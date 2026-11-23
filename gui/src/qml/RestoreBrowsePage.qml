// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// Restore area: browsing one snapshot as a folder list with a path row (no tree). Return opens
// a folder, Space marks an entry for restore; the path row leads back to every folder above.
// The daemon lists each folder once; the list loads page by page while scrolling.
Kirigami.ScrollablePage {
    id: page
    objectName: "restoreBrowsePage"
    title: snapshotTitle

    required property var rootWindow
    required property var appBackend
    property string destinationId: ""
    property string snapshotId: ""
    property string startDir: "/"
    // Date and time of the snapshot, shown in the breadcrumb.
    property string snapshotTitle: ""
    // An entry of `startDir` to show selected once it is listed (a search hit).
    property string focusPath: ""

    // The list whose selected row gets the focus when the page is shown again.
    readonly property var primaryList: browseListView
    readonly property alias browseModel: snapshotBrowseModel

    signal restoreRequested(var destinationId, var snapshotId, var selection)

    // Lists `dir`; the filter starts empty in every folder, the selection stays.
    function openDir(dir) {
        filterField.resetFilter();
        browseListView.currentIndex = -1;
        snapshotBrowseModel.open(destinationId, snapshotId, dir);
    }

    // Opens the folder at `row`, or marks the file there.
    function activateRow(row) {
        var path = snapshotBrowseModel.data(snapshotBrowseModel.index(row, 0), 0);
        var isDir = snapshotBrowseModel.data(snapshotBrowseModel.index(row, 0), 2);
        if (isDir) {
            openDir(path);
        } else {
            snapshotBrowseModel.toggleSelected(row);
        }
    }

    Component.onCompleted: openDir(startDir)

    // A snapshot whose backed-up folder is not in its tree (made with a relative path or of a
    // single file) opens at the top instead.
    Connections {
        target: snapshotBrowseModel
        function onStateChanged() {
            if (snapshotBrowseModel.state !== "done" || snapshotBrowseModel.dir !== page.startDir) {
                return;
            }
            if (!snapshotBrowseModel.dir_exists && page.startDir !== "/") {
                page.openDir("/");
            } else if (page.focusPath !== "") {
                var row = snapshotBrowseModel.rowOfPath(page.focusPath);
                page.focusPath = "";
                if (row >= 0) {
                    browseListView.currentIndex = row;
                    browseListView.positionViewAtIndex(row, ListView.Center);
                }
            }
        }
    }
    Component.onDestruction: snapshotBrowseModel.cancel()
    // A page taken off the stack is hidden, not destroyed: stop listing there, too.
    onVisibleChanged: {
        if (!visible) {
            snapshotBrowseModel.cancel();
        } else if (snapshotBrowseModel.state === "cancelled") {
            openDir(snapshotBrowseModel.dir);
        }
    }

    SnapshotBrowseModel {
        id: snapshotBrowseModel
        objectName: "snapshotBrowseModel"
    }

    actions: [
        Kirigami.Action {
            objectName: "restoreSelectionAction"
            icon.name: "document-revert"
            text: i18n("Restore Selection…")
            enabled: snapshotBrowseModel.selection_count > 0
            tooltip: enabled ? "" : i18n("Select files or folders first")
            onTriggered: page.restoreRequested(page.destinationId, page.snapshotId,
                snapshotBrowseModel.selectedPaths())
        }
    ]

    header: Controls.ToolBar {
        leftPadding: Kirigami.Units.largeSpacing
        rightPadding: Kirigami.Units.largeSpacing
        contentItem: ColumnLayout {
            spacing: Kirigami.Units.smallSpacing

            // One line, like Dolphin's location bar: a path longer than the page scrolls
            // sideways and shows its end, the open folder, so the list keeps its height.
            Flickable {
                id: pathFlick
                objectName: "browsePathFlick"
                Layout.fillWidth: true
                Layout.preferredHeight: pathRow.implicitHeight
                contentWidth: pathRow.implicitWidth
                contentHeight: pathRow.implicitHeight
                flickableDirection: Flickable.HorizontalFlick
                boundsBehavior: Flickable.StopAtBounds
                clip: true

                // Scrolls to the open folder and starts the row with a whole folder (its «›»
                // first) rather than a cut one. The space this leaves after the open folder
                // becomes a right margin, so the position stays within the bounds and the
                // Flickable does not pull it back into a folder name.
                function showEnd() {
                    rightMargin = 0;
                    const end = Math.max(0, contentWidth - width);
                    let x = end;
                    if (end > 0) {
                        for (let i = 0; i < pathRow.children.length; ++i) {
                            const crumb = pathRow.children[i];
                            if (crumb.width > 0 && crumb.x >= end) {
                                x = crumb.x;
                                break;
                            }
                        }
                    }
                    rightMargin = x - end;
                    contentX = x;
                }
                function ensureVisible(item) {
                    const x = item.mapToItem(pathRow, 0, 0).x;
                    if (x < contentX) {
                        contentX = x;
                    } else if (x + item.width > contentX + width) {
                        contentX = Math.min(Math.max(0, contentWidth + rightMargin - width),
                            x + item.width - width);
                    }
                }
                onContentWidthChanged: showEnd()
                onWidthChanged: showEnd()

                Row {
                    id: pathRow
                    objectName: "browsePathRow"
                    spacing: Kirigami.Units.smallSpacing

                    Repeater {
                        model: snapshotBrowseModel.segmentPaths(snapshotBrowseModel.dir)
                        delegate: Row {
                            id: crumb
                            required property int index
                            required property string modelData
                            readonly property string crumbName:
                                snapshotBrowseModel.segmentNames(snapshotBrowseModel.dir)[index]
                            readonly property bool isCurrent: modelData === snapshotBrowseModel.dir
                            spacing: Kirigami.Units.smallSpacing

                            Controls.Label {
                                visible: crumb.index > 0
                                anchors.verticalCenter: parent.verticalCenter
                                text: "›"
                                opacity: 0.7
                            }
                            Controls.ToolButton {
                                id: crumbButton
                                objectName: "browsePathButton_" + crumb.index
                                readonly property string path: crumb.modelData
                                readonly property string label: crumb.index === 0 ? i18n("Top folder") : crumb.crumbName
                                // The button's own text and icon stay empty: the desktop style paints
                                // them in its background, beneath the content item below, so every
                                // folder would be drawn twice. The content item draws the folder.
                                padding: Kirigami.Units.smallSpacing
                                checkable: false
                                font.bold: crumb.isCurrent
                                activeFocusOnTab: true
                                Accessible.name: label
                                Controls.ToolTip.visible: crumb.index === 0 && hovered
                                Controls.ToolTip.text: label
                                // Folder names from the snapshot are shown literally, never as rich text.
                                contentItem: RowLayout {
                                    spacing: 0
                                    Kirigami.Icon {
                                        visible: crumb.index === 0
                                        source: "drive-harddisk-root"
                                        Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                                        Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                                    }
                                    Controls.Label {
                                        visible: crumb.index > 0
                                        Layout.fillWidth: true
                                        // As tall as the top folder icon, so all folders share one line.
                                        Layout.minimumHeight: Kirigami.Units.iconSizes.smallMedium
                                        horizontalAlignment: Text.AlignHCenter
                                        verticalAlignment: Text.AlignVCenter
                                        text: crumbButton.label
                                        textFormat: Text.PlainText
                                        font: crumbButton.font
                                    }
                                }
                                onClicked: {
                                    if (!crumb.isCurrent) page.openDir(crumb.modelData);
                                }
                                onActiveFocusChanged: if (activeFocus) pathFlick.ensureVisible(crumb)
                                FocusRing { target: crumbButton }
                            }
                        }
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing

                FilterField {
                    id: filterField
                    objectName: "browseFilterField"
                    Layout.fillWidth: true
                    onFilterRequested: function(text) { snapshotBrowseModel.setFilter(text); }
                }
                Controls.Label {
                    objectName: "browseCountLabel"
                    visible: snapshotBrowseModel.state === "done" && snapshotBrowseModel.dir_exists
                    text: snapshotBrowseModel.count_text
                    opacity: 0.8
                }
            }
        }
    }

    footer: Controls.ToolBar {
        visible: snapshotBrowseModel.selection_count > 0
        contentItem: RowLayout {
            spacing: Kirigami.Units.mediumSpacing
            Controls.Label {
                objectName: "browseSelectionLabel"
                Layout.fillWidth: true
                text: snapshotBrowseModel.selection_text
                textFormat: Text.PlainText
                elide: Text.ElideRight
            }
            Controls.Button {
                objectName: "showSelectionButton"
                text: i18n("Show Selection")
                onClicked: selectionDialog.open()
            }
            Controls.Button {
                objectName: "clearSelectionButton"
                text: i18n("Clear Selection")
                onClicked: snapshotBrowseModel.clearSelection()
            }
        }
    }

    ListView {
        id: browseListView
        objectName: "browseListView"
        model: snapshotBrowseModel
        keyNavigationEnabled: true
        reuseItems: true

        onActiveFocusChanged: {
            if (activeFocus && count > 0 && currentIndex < 0) {
                currentIndex = 0;
            }
        }

        header: RowLayout {
            width: ListView.view.width
            spacing: Kirigami.Units.mediumSpacing
            Item { Layout.preferredWidth: Kirigami.Units.gridUnit * 3 }
            Controls.Label {
                Layout.fillWidth: true
                text: i18n("Name")
                font.bold: true
            }
            Controls.Label {
                Layout.preferredWidth: Kirigami.Units.gridUnit * 5
                horizontalAlignment: Text.AlignRight
                text: i18n("Size")
                font.bold: true
            }
            Controls.Label {
                Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                text: i18n("Modified")
                font.bold: true
            }
        }

        QueryStatus {
            queryModel: snapshotBrowseModel
            namePrefix: "browse"
            onRetryRequested: page.openDir(snapshotBrowseModel.dir)
        }

        Kirigami.PlaceholderMessage {
            objectName: "browseEmptyMessage"
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 4
            visible: snapshotBrowseModel.state === "done" && !snapshotBrowseModel.too_large
                && browseListView.count === 0
            icon.name: "folder"
            text: !snapshotBrowseModel.dir_exists ? i18n("This folder is not in the snapshot.")
                : filterField.text !== "" ? Texts.label("no_entries_match_filter")
                : i18n("This folder is empty in the snapshot.")
        }

        delegate: Controls.ItemDelegate {
            id: entryRow
            objectName: "browseEntryRow"
            required property int index
            required property string path
            required property string name
            required property bool isDir
            required property string iconName
            required property string sizeText
            required property string modifiedText
            required property bool selected

            width: ListView.view.width
            activeFocusOnTab: true
            highlighted: ListView.isCurrentItem
            Accessible.name: name
            Accessible.description: selected ? i18n("Marked for restore") : ""

            onClicked: {
                ListView.view.currentIndex = index;
                page.activateRow(index);
            }

            Keys.onPressed: function(event) {
                if (event.key === Qt.Key_Space) {
                    snapshotBrowseModel.toggleSelected(entryRow.index);
                    event.accepted = true;
                } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                    if (entryRow.isDir) page.openDir(entryRow.path);
                    event.accepted = true;
                }
            }

            FocusRing {
                target: entryRow
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.mediumSpacing

                Controls.CheckBox {
                    objectName: "browseEntryCheck"
                    checked: entryRow.selected
                    focusPolicy: Qt.NoFocus
                    Accessible.name: i18n("Mark %1 for restore", entryRow.name)
                    onToggled: snapshotBrowseModel.toggleSelected(entryRow.index)
                }
                Kirigami.Icon {
                    source: entryRow.iconName
                    Layout.preferredWidth: Kirigami.Units.iconSizes.small
                    Layout.preferredHeight: Kirigami.Units.iconSizes.small
                }
                Controls.Label {
                    Layout.fillWidth: true
                    text: entryRow.name
                    textFormat: Text.PlainText
                    elide: Text.ElideMiddle
                    color: entryRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                }
                Controls.Label {
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 5
                    horizontalAlignment: Text.AlignRight
                    text: entryRow.sizeText
                    color: entryRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                }
                Controls.Label {
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                    text: entryRow.modifiedText
                    color: entryRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                }
            }
        }
    }

    Kirigami.Dialog {
        id: selectionDialog
        objectName: "selectionDialog"
        title: i18n("Selection")
        standardButtons: Kirigami.Dialog.Close
        preferredWidth: Kirigami.Units.gridUnit * 28

        ListView {
            implicitHeight: Math.min(contentHeight, Kirigami.Units.gridUnit * 16)
            clip: true
            model: selectionDialog.visible ? snapshotBrowseModel.selectedPaths() : []
            delegate: Controls.Label {
                required property string modelData
                width: ListView.view.width
                text: modelData
                textFormat: Text.PlainText
                elide: Text.ElideMiddle
                padding: Kirigami.Units.smallSpacing
            }
        }
    }
}
