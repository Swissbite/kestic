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

    // The list whose selected row gets the focus when the page is shown again.
    readonly property var primaryList: browseListView
    readonly property alias browseModel: snapshotBrowseModel

    signal restoreRequested(var destinationId, var snapshotId, var selection)

    // Lists `dir`; the filter starts empty in every folder, the selection stays.
    function openDir(dir) {
        filterDelay.stop();
        filterField.text = "";
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
            if (snapshotBrowseModel.state === "done" && !snapshotBrowseModel.dir_exists
                    && snapshotBrowseModel.dir === page.startDir && page.startDir !== "/") {
                page.openDir("/");
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

    Timer {
        interval: 150
        repeat: true
        running: snapshotBrowseModel.state === "running"
        onTriggered: snapshotBrowseModel.poll()
    }

    Timer {
        id: filterDelay
        interval: 300
        onTriggered: snapshotBrowseModel.setFilter(filterField.text)
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
        contentItem: ColumnLayout {
            spacing: Kirigami.Units.smallSpacing

            Flow {
                id: pathRow
                objectName: "browsePathRow"
                Layout.fillWidth: true
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
                            icon.name: crumb.index === 0 ? "drive-harddisk-root" : ""
                            text: crumb.index === 0 ? i18n("Top folder") : crumb.crumbName
                            display: crumb.index === 0 ? Controls.AbstractButton.IconOnly
                                : Controls.AbstractButton.TextOnly
                            checkable: false
                            font.bold: crumb.isCurrent
                            activeFocusOnTab: true
                            Accessible.name: text
                            Controls.ToolTip.visible: crumb.index === 0 && hovered
                            Controls.ToolTip.text: text
                            // Folder names from the snapshot are shown literally, never as rich text.
                            contentItem: RowLayout {
                                spacing: 0
                                Kirigami.Icon {
                                    visible: crumb.index === 0
                                    source: crumbButton.icon.name
                                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                                }
                                Controls.Label {
                                    visible: crumb.index > 0
                                    text: crumbButton.text
                                    textFormat: Text.PlainText
                                    font: crumbButton.font
                                }
                            }
                            onClicked: {
                                if (!crumb.isCurrent) page.openDir(crumb.modelData);
                            }
                            FocusRing { target: crumbButton }
                        }
                    }
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing

                Kirigami.SearchField {
                    id: filterField
                    objectName: "browseFilterField"
                    Layout.fillWidth: true
                    placeholderText: i18n("Filter")
                    Accessible.name: i18n("Filter")
                    onTextChanged: filterDelay.restart()
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

        Kirigami.LoadingPlaceholder {
            anchors.centerIn: parent
            visible: snapshotBrowseModel.state === "running"
        }

        Kirigami.InlineMessage {
            objectName: "browseErrorMessage"
            anchors.top: parent.top
            width: parent.width
            type: Kirigami.MessageType.Error
            visible: snapshotBrowseModel.state === "error"
            text: snapshotBrowseModel.error_text
            actions: [
                Kirigami.Action {
                    text: Texts.label("retry")
                    onTriggered: page.openDir(snapshotBrowseModel.dir)
                }
            ]
        }

        Kirigami.PlaceholderMessage {
            objectName: "browseTooLargeMessage"
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 4
            visible: snapshotBrowseModel.state === "done" && snapshotBrowseModel.too_large
            icon.name: "dialog-information"
            text: snapshotBrowseModel.too_large_text
        }

        Kirigami.PlaceholderMessage {
            objectName: "browseEmptyMessage"
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 4
            visible: snapshotBrowseModel.state === "done" && !snapshotBrowseModel.too_large
                && browseListView.count === 0
            icon.name: "folder"
            text: !snapshotBrowseModel.dir_exists ? i18n("This folder is not in the snapshot.")
                : filterField.text !== "" ? i18n("No entries match the filter.")
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
                anchors.margins: entryRow.highlighted ? 2 : 0
                ringColor: entryRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.highlightColor
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
