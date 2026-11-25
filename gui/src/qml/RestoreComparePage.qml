// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// Restore area: the differences from one snapshot to another, each changed path with the kind
// of its change (icon and text). The chips filter by kind of change and name how many there
// are, the filter field by text in the path. The daemon compares once; the changes load page by
// page while scrolling.
Kirigami.ScrollablePage {
    id: page
    objectName: "restoreComparePage"
    title: i18n("Compare")

    required property var rootWindow
    required property var appBackend
    property string destinationId: ""
    property string fromSnapshotId: ""
    property string toSnapshotId: ""
    // Date and time of both snapshots, shown above the changes.
    property string fromSnapshotText: ""
    property string toSnapshotText: ""

    // The list whose selected row gets the focus when the page is shown again.
    readonly property var primaryList: diffListView
    readonly property alias diffModel: snapshotDiffModel
    // Side padding of a row (that of the desktop style's list rows), also used by the headings.
    readonly property int rowPadding: Kirigami.Units.mediumSpacing + Kirigami.Units.smallSpacing
    readonly property int changeColumnWidth: Kirigami.Units.gridUnit * 9

    // The user cancelled the comparison; a hidden page that comes back does not start it again.
    property bool cancelledByUser: false

    function runCompare() {
        cancelledByUser = false;
        filterField.resetFilter();
        kindChips.uncheckAll();
        diffListView.currentIndex = -1;
        snapshotDiffModel.compare(destinationId, fromSnapshotId, toSnapshotId);
    }

    Component.onCompleted: runCompare()
    Component.onDestruction: snapshotDiffModel.cancel()
    // A page taken off the stack is hidden, not destroyed: stop comparing there, too.
    onVisibleChanged: {
        if (!visible) {
            snapshotDiffModel.cancel();
        } else if (snapshotDiffModel.state === "cancelled" && !cancelledByUser) {
            runCompare();
        }
    }

    SnapshotDiffModel {
        id: snapshotDiffModel
        objectName: "snapshotDiffModel"
    }

    header: Controls.ToolBar {
        leftPadding: Kirigami.Units.largeSpacing
        rightPadding: Kirigami.Units.largeSpacing
        contentItem: ColumnLayout {
            spacing: Kirigami.Units.smallSpacing

            Controls.Label {
                objectName: "compareSnapshotsLabel"
                Layout.fillWidth: true
                text: i18n("From %1 to %2", page.fromSnapshotText, page.toSnapshotText)
                elide: Text.ElideRight
            }

            // On a narrow page the filter gets a line of its own below the number of changes.
            GridLayout {
                id: countRow
                Layout.fillWidth: true
                columns: page.width < Kirigami.Units.gridUnit * 30 ? 1 : 2
                columnSpacing: Kirigami.Units.mediumSpacing
                rowSpacing: Kirigami.Units.smallSpacing
                visible: snapshotDiffModel.state === "done" && snapshotDiffModel.total > 0

                Controls.Label {
                    objectName: "compareCountLabel"
                    Layout.fillWidth: true
                    text: snapshotDiffModel.count_text
                    textFormat: Text.PlainText
                    wrapMode: Text.Wrap
                    font.bold: true
                    opacity: 0.8
                }
                FilterField {
                    id: filterField
                    objectName: "compareFilterField"
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 12
                    Layout.fillWidth: countRow.columns === 1
                    visible: !snapshotDiffModel.too_large
                    onFilterRequested: function(text) { snapshotDiffModel.setFilter(text); }
                }
            }

            KindChips {
                id: kindChips
                objectName: "compareKindChips"
                Layout.fillWidth: true
                visible: snapshotDiffModel.state === "done" && !snapshotDiffModel.too_large
                    && snapshotDiffModel.kind_codes.length > 0
                listModel: snapshotDiffModel
                namePrefix: "compareKindChip_"
                onKindsToggled: diffListView.currentIndex = -1
            }
        }
    }

    KeyboardList {
        id: diffListView
        objectName: "diffListView"
        model: snapshotDiffModel
        keyNavigationEnabled: true
        reuseItems: true

        onActiveFocusChanged: {
            if (activeFocus && count > 0 && currentIndex < 0) {
                currentIndex = 0;
            }
        }

        // The column headings with the paddings and column widths of the rows below.
        header: Item {
            width: ListView.view.width
            implicitHeight: headingRow.implicitHeight
            visible: diffListView.count > 0

            RowLayout {
                id: headingRow
                anchors.fill: parent
                anchors.leftMargin: page.rowPadding
                anchors.rightMargin: page.rowPadding
                spacing: Kirigami.Units.mediumSpacing
                Controls.Label {
                    Layout.preferredWidth: page.changeColumnWidth
                    Layout.minimumWidth: page.changeColumnWidth
                    Layout.maximumWidth: page.changeColumnWidth
                    text: i18n("Change")
                    font.bold: true
                }
                Controls.Label {
                    Layout.fillWidth: true
                    text: i18n("Path")
                    font.bold: true
                }
            }
        }

        QueryStatus {
            queryModel: snapshotDiffModel
            namePrefix: "compare"
            busyText: i18n("Comparing…")
            cancelledText: i18n("Comparison cancelled")
            onCancelRequested: {
                page.cancelledByUser = true;
                snapshotDiffModel.cancel();
            }
            onRetryRequested: page.runCompare()
        }

        Kirigami.PlaceholderMessage {
            objectName: "compareNoDifferencesMessage"
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 4
            visible: snapshotDiffModel.state === "done" && !snapshotDiffModel.too_large
                && diffListView.count === 0
            icon.name: snapshotDiffModel.identical ? "emblem-ok-symbolic" : "edit-find"
            text: snapshotDiffModel.identical ? i18n("No differences")
                : Texts.label("no_entries_match_filter")
        }

        delegate: Controls.ItemDelegate {
            id: changeRow
            objectName: "diffChangeRow"
            required property int index
            required property string path
            required property string change
            required property string changeText
            required property string iconName

            width: ListView.view.width
            leftPadding: page.rowPadding
            rightPadding: page.rowPadding
            focusPolicy: Qt.ClickFocus
            highlighted: ListView.isCurrentItem
            Accessible.name: path
            Accessible.description: changeText

            onClicked: ListView.view.currentIndex = index

            FocusRing {
                target: changeRow
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.mediumSpacing

                RowLayout {
                    Layout.preferredWidth: page.changeColumnWidth
                    Layout.minimumWidth: page.changeColumnWidth
                    Layout.maximumWidth: page.changeColumnWidth
                    spacing: Kirigami.Units.smallSpacing

                    Kirigami.Icon {
                        source: changeRow.iconName
                        selected: changeRow.highlighted
                        Layout.preferredWidth: Kirigami.Units.iconSizes.small
                        Layout.preferredHeight: Kirigami.Units.iconSizes.small
                    }
                    Controls.Label {
                        objectName: "diffChangeText"
                        Layout.fillWidth: true
                        text: changeRow.changeText
                        elide: Text.ElideRight
                        color: changeRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                    }
                }
                // Paths from the snapshot are shown literally, never as rich text.
                Controls.Label {
                    Layout.fillWidth: true
                    text: changeRow.path
                    textFormat: Text.PlainText
                    elide: Text.ElideMiddle
                    color: changeRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                }
            }
        }
    }
}
