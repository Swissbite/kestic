// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// Restore area: the hits of a search in the snapshots of one backup, each with its path and the
// date of its snapshot. «Show», a click or Return on a hit opens the hit in the folder list of its
// snapshot. The daemon searches once; the hits load page by page while scrolling.
Kirigami.ScrollablePage {
    id: page
    objectName: "restoreSearchPage"
    title: i18n("Search")

    required property var rootWindow
    required property var appBackend
    property string destinationId: ""
    property string backupId: ""
    property string searchText: ""

    // The list whose selected row gets the focus when the page is shown again.
    readonly property var primaryList: searchListView
    readonly property alias searchModel: snapshotSearchModel

    signal showRequested(var destinationId, var snapshotId, var folder, var path, var snapshotText)

    // The user cancelled the search; a hidden page that comes back does not start it again.
    property bool cancelledByUser: false

    function runSearch(text) {
        cancelledByUser = false;
        filterField.resetFilter();
        searchListView.currentIndex = -1;
        snapshotSearchModel.search(destinationId, backupId, text);
    }

    function showHit(row) {
        var hit = snapshotSearchModel.index(row, 0);
        page.showRequested(page.destinationId,
            snapshotSearchModel.data(hit, 1),
            snapshotSearchModel.data(hit, 3),
            snapshotSearchModel.data(hit, 0),
            snapshotSearchModel.data(hit, 2));
    }

    Component.onCompleted: {
        searchField.text = searchText;
        runSearch(searchText);
    }
    Component.onDestruction: snapshotSearchModel.cancel()
    // A page taken off the stack is hidden, not destroyed: stop searching there, too.
    onVisibleChanged: {
        if (!visible) {
            snapshotSearchModel.cancel();
        } else if (snapshotSearchModel.state === "cancelled" && !cancelledByUser) {
            runSearch(snapshotSearchModel.search_text);
        }
    }

    SnapshotSearchModel {
        id: snapshotSearchModel
        objectName: "snapshotSearchModel"
    }

    header: Controls.ToolBar {
        leftPadding: Kirigami.Units.largeSpacing
        rightPadding: Kirigami.Units.largeSpacing
        contentItem: ColumnLayout {
            spacing: Kirigami.Units.smallSpacing

            Kirigami.SearchField {
                id: searchField
                objectName: "searchPageField"
                Layout.fillWidth: true
                placeholderText: i18n("Search text")
                Accessible.name: i18n("Search text")
                // Ctrl+F belongs to the search field of the snapshot list.
                focusSequences: []
                autoAccept: false
                onAccepted: {
                    if (text.trim() !== "") page.runSearch(text);
                }
            }

            // On a narrow page the filter gets a line of its own below the number of hits.
            GridLayout {
                id: countRow
                Layout.fillWidth: true
                columns: page.width < Kirigami.Units.gridUnit * 30 ? 1 : 2
                columnSpacing: Kirigami.Units.mediumSpacing
                rowSpacing: Kirigami.Units.smallSpacing
                visible: snapshotSearchModel.state === "done" && snapshotSearchModel.total > 0

                Controls.Label {
                    objectName: "searchCountLabel"
                    Layout.fillWidth: true
                    text: snapshotSearchModel.count_text
                    textFormat: Text.PlainText
                    wrapMode: Text.Wrap
                    font.bold: true
                    opacity: 0.8
                }
                FilterField {
                    id: filterField
                    objectName: "searchFilterField"
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 12
                    Layout.fillWidth: countRow.columns === 1
                    visible: !snapshotSearchModel.too_large
                    onFilterRequested: function(text) { snapshotSearchModel.setFilter(text); }
                }
            }
        }
    }

    KeyboardList {
        id: searchListView
        objectName: "searchListView"
        model: snapshotSearchModel
        keyNavigationEnabled: true
        reuseItems: true

        onActiveFocusChanged: {
            if (activeFocus && count > 0 && currentIndex < 0) {
                currentIndex = 0;
            }
        }

        QueryStatus {
            queryModel: snapshotSearchModel
            namePrefix: "search"
            busyText: i18n("Searching…")
            cancelledText: i18n("Search cancelled")
            onCancelRequested: {
                page.cancelledByUser = true;
                snapshotSearchModel.cancel();
            }
            onRetryRequested: page.runSearch(snapshotSearchModel.search_text)
        }

        // Like a PlaceholderMessage, but the search text in it is shown literally, never as
        // rich text.
        ColumnLayout {
            objectName: "searchNoHitsMessage"
            readonly property string text: snapshotSearchModel.total === 0
                ? snapshotSearchModel.no_hits_text : Texts.label("no_entries_match_filter")
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 4
            visible: snapshotSearchModel.state === "done" && !snapshotSearchModel.too_large
                && searchListView.count === 0
            spacing: Kirigami.Units.largeSpacing

            Kirigami.Icon {
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: Kirigami.Units.iconSizes.huge
                Layout.preferredHeight: Kirigami.Units.iconSizes.huge
                source: "edit-find"
                opacity: 0.5
            }
            Kirigami.Heading {
                Layout.fillWidth: true
                level: 2
                text: parent.text
                textFormat: Text.PlainText
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                opacity: 0.5
            }
        }

        delegate: Controls.ItemDelegate {
            id: hitRow
            objectName: "searchHitRow"
            required property int index
            required property string path
            required property string iconName
            required property string subtitle

            width: ListView.view.width
            focusPolicy: Qt.ClickFocus
            highlighted: ListView.isCurrentItem
            Accessible.name: path
            Accessible.description: subtitle

            // Return activates the focused row like a click (Main.qml).
            onClicked: {
                ListView.view.currentIndex = index;
                page.showHit(index);
            }

            FocusRing {
                target: hitRow
            }

            contentItem: RowLayout {
                spacing: Kirigami.Units.mediumSpacing

                Kirigami.Icon {
                    source: hitRow.iconName
                    Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
                    Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2
                    // Paths from the snapshot are shown literally, never as rich text.
                    Controls.Label {
                        Layout.fillWidth: true
                        text: hitRow.path
                        textFormat: Text.PlainText
                        elide: Text.ElideMiddle
                        color: hitRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                    }
                    Controls.Label {
                        Layout.fillWidth: true
                        text: hitRow.subtitle
                        textFormat: Text.PlainText
                        elide: Text.ElideRight
                        opacity: 0.75
                        color: hitRow.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                    }
                }
                Controls.Button {
                    objectName: "searchShowButton"
                    text: i18n("Show")
                    icon.name: "go-next-symbolic"
                    focusPolicy: Qt.NoFocus
                    Accessible.name: i18n("Show %1", hitRow.path)
                    onClicked: page.showHit(hitRow.index)
                }
            }
        }
    }
}
