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

    // The kinds of change whose chip is checked; none shows all changes.
    property var checkedKinds: []
    // The user cancelled the comparison; a hidden page that comes back does not start it again.
    property bool cancelledByUser: false

    function runCompare() {
        cancelledByUser = false;
        filterDelay.stop();
        filterField.text = "";
        checkedKinds = [];
        diffListView.currentIndex = -1;
        snapshotDiffModel.compare(destinationId, fromSnapshotId, toSnapshotId);
    }

    function toggleKind(code, checked) {
        var kinds = checkedKinds.filter(function(k) { return k !== code; });
        if (checked) kinds.push(code);
        checkedKinds = kinds;
        diffListView.currentIndex = -1;
        snapshotDiffModel.setKinds(kinds);
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

    Timer {
        interval: 150
        repeat: true
        running: snapshotDiffModel.state === "running"
        onTriggered: snapshotDiffModel.poll()
    }

    Timer {
        id: filterDelay
        interval: 300
        onTriggered: snapshotDiffModel.setFilter(filterField.text)
    }

    header: Controls.ToolBar {
        contentItem: ColumnLayout {
            spacing: Kirigami.Units.smallSpacing

            Controls.Label {
                objectName: "compareSnapshotsLabel"
                Layout.fillWidth: true
                text: i18n("From %1 to %2", page.fromSnapshotText, page.toSnapshotText)
                elide: Text.ElideRight
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: Kirigami.Units.mediumSpacing
                visible: snapshotDiffModel.state === "done" && snapshotDiffModel.total > 0

                Controls.Label {
                    objectName: "compareCountLabel"
                    Layout.fillWidth: true
                    text: snapshotDiffModel.count_text
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    font.bold: true
                    opacity: 0.8
                }
                Kirigami.SearchField {
                    id: filterField
                    objectName: "compareFilterField"
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 12
                    visible: !snapshotDiffModel.too_large
                    placeholderText: Texts.label("filter")
                    Accessible.name: Texts.label("filter")
                    // Ctrl+F belongs to the search field of the snapshot list.
                    focusSequences: []
                    onTextChanged: filterDelay.restart()
                }
            }

            Flow {
                objectName: "compareKindChips"
                Layout.fillWidth: true
                spacing: Kirigami.Units.smallSpacing
                visible: snapshotDiffModel.state === "done" && !snapshotDiffModel.too_large
                    && snapshotDiffModel.kind_codes.length > 0

                Repeater {
                    model: snapshotDiffModel.kind_codes
                    delegate: Kirigami.Chip {
                        id: kindChip
                        required property int index
                        required property string modelData
                        objectName: "compareKindChip_" + modelData
                        text: snapshotDiffModel.kind_texts[index]
                        checkable: true
                        closable: false
                        checked: page.checkedKinds.indexOf(modelData) >= 0
                        Accessible.name: text
                        onToggled: {
                            page.toggleKind(kindChip.modelData, kindChip.checked);
                            kindChip.forceActiveFocus(Qt.TabFocusReason);
                        }

                        FocusRing {
                            target: kindChip
                            radius: Math.round(kindChip.height / 2)
                            ringColor: kindChip.checked ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.highlightColor
                        }
                    }
                }
            }
        }
    }

    ListView {
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

        header: RowLayout {
            width: ListView.view.width
            visible: diffListView.count > 0
            spacing: Kirigami.Units.mediumSpacing
            Controls.Label {
                Layout.preferredWidth: Kirigami.Units.gridUnit * 9
                leftPadding: Kirigami.Units.largeSpacing
                text: i18n("Change")
                font.bold: true
            }
            Controls.Label {
                Layout.fillWidth: true
                text: i18n("Path")
                font.bold: true
            }
        }

        ColumnLayout {
            objectName: "compareBusy"
            anchors.centerIn: parent
            visible: snapshotDiffModel.state === "running"
            spacing: Kirigami.Units.largeSpacing

            Controls.BusyIndicator {
                Layout.alignment: Qt.AlignHCenter
                running: parent.visible
                Accessible.name: i18n("Comparing…")
            }
            Controls.Button {
                objectName: "compareCancelButton"
                Layout.alignment: Qt.AlignHCenter
                text: Texts.label("cancel")
                icon.name: "process-stop"
                onClicked: {
                    page.cancelledByUser = true;
                    snapshotDiffModel.cancel();
                }
            }
        }

        Kirigami.PlaceholderMessage {
            objectName: "compareCancelledMessage"
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 4
            visible: snapshotDiffModel.state === "cancelled"
            icon.name: "process-stop"
            text: i18n("Comparison cancelled")
            helpfulAction: Kirigami.Action {
                text: Texts.label("retry")
                onTriggered: page.runCompare()
            }
        }

        Kirigami.InlineMessage {
            objectName: "compareErrorMessage"
            anchors.top: parent.top
            width: parent.width
            type: Kirigami.MessageType.Error
            visible: snapshotDiffModel.state === "error"
            text: snapshotDiffModel.error_text
            actions: [
                Kirigami.Action {
                    text: Texts.label("retry")
                    onTriggered: page.runCompare()
                }
            ]
        }

        Kirigami.PlaceholderMessage {
            objectName: "compareTooLargeMessage"
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 4
            visible: snapshotDiffModel.state === "done" && snapshotDiffModel.too_large
            icon.name: "dialog-information"
            text: snapshotDiffModel.too_large_text
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
            activeFocusOnTab: true
            highlighted: ListView.isCurrentItem
            Accessible.name: path
            Accessible.description: changeText

            onClicked: ListView.view.currentIndex = index

            FocusRing {
                target: changeRow
                anchors.margins: changeRow.highlighted ? 2 : 0
                ringColor: changeRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.highlightColor
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.mediumSpacing

                RowLayout {
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 9
                    Layout.maximumWidth: Kirigami.Units.gridUnit * 9
                    spacing: Kirigami.Units.smallSpacing

                    Kirigami.Icon {
                        source: changeRow.iconName
                        color: changeRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
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
