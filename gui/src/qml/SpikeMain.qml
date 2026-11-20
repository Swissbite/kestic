// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ApplicationWindow {
    readonly property SharedTexts sharedTexts: SharedTexts {}
    id: root
    title: "Kestic"
    width: 900
    height: 600
    minimumWidth: Math.round(Kirigami.Units.gridUnit * 26)
    minimumHeight: Math.round(Kirigami.Units.gridUnit * 24)
    visible: true

    property int activeTab: 2
    property int backupNowTriggerCount: 0
    readonly property bool narrowMode: width < Kirigami.Units.gridUnit * 35

    function triggerBackupNow() {
        backupNowTriggerCount += 1;
        showPassiveNotification(i18n("Back Up Now triggered (Ctrl+Shift+B)"));
    }

    function pushFromComponent(comp, props) {
        return pageStack.push(comp.createObject(pageStack, props || {}));
    }

    function replaceFromComponent(comp, props) {
        return pageStack.replace(comp.createObject(pageStack, props || {}));
    }

    function openDeepSubpageForSmoke() {
        while (pageStack.depth > 1) {
            pageStack.pop();
        }
        pushFromComponent(backupDetailPage, {});
        pushFromComponent(retentionPage, {});
    }

    function resizeToMinWidthForSmoke() {
        width = Math.round(Kirigami.Units.gridUnit * 26);
        height = Math.round(Kirigami.Units.gridUnit * 24);
    }

    Shortcut {
        id: backupNowShortcut
        objectName: "backupNowShortcut"
        sequence: "Ctrl+Shift+B"
        context: Qt.ApplicationShortcut
        onActivated: root.triggerBackupNow()
    }

    SpikeListModel {
        id: spikeModel
        objectName: "spikeModel"
    }

    property list<Kirigami.Action> navActions: [
        Kirigami.Action {
            icon.name: "dashboard-show"
            text: i18n("Overview")
            checked: root.activeTab === 0
            onTriggered: {
                root.activeTab = 0;
                root.replaceFromComponent(overviewPage, {});
            }
        },
        Kirigami.Action {
            icon.name: "document-revert"
            text: i18n("Restore")
            checked: root.activeTab === 1
            onTriggered: {
                root.activeTab = 1;
                root.replaceFromComponent(placeholderTabPage, { pageTitle: i18n("Restore") });
            }
        },
        Kirigami.Action {
            icon.name: "folder-sync"
            text: i18n("Backups")
            checked: root.activeTab === 2
            onTriggered: {
                root.activeTab = 2;
                root.replaceFromComponent(backupsListComponent, {});
                if (!root.narrowMode) {
                    root.pushFromComponent(backupDetailPage, {});
                }
            }
        },
        Kirigami.Action {
            icon.name: "network-server"
            text: i18n("Destinations")
            checked: root.activeTab === 3
            onTriggered: {
                root.activeTab = 3;
                root.replaceFromComponent(placeholderTabPage, { pageTitle: i18n("Destinations") });
            }
        },
        Kirigami.Action {
            icon.name: "view-history"
            text: i18n("History")
            checked: root.activeTab === 4
            onTriggered: {
                root.activeTab = 4;
                root.replaceFromComponent(placeholderTabPage, { pageTitle: i18n("History") });
            }
        }
    ]

    header: Kirigami.NavigationTabBar {
        id: topNavBar
        objectName: "topNavBar"
        visible: !root.narrowMode
        height: visible ? implicitHeight : 0
        position: Controls.ToolBar.Header
        actions: root.navActions
    }

    footer: Kirigami.NavigationTabBar {
        id: bottomNavBar
        objectName: "bottomNavBar"
        visible: root.narrowMode
        height: visible ? implicitHeight : 0
        position: Controls.ToolBar.Footer
        actions: root.navActions
    }

    pageStack.globalToolBar.style: Kirigami.ApplicationHeaderStyle.Breadcrumb
    pageStack.initialPage: Kirigami.ScrollablePage {
        id: backupsListPage
        title: i18n("Backups")

        ListView {
            id: backupList
            objectName: "spikeBackupList"
            model: spikeModel
            spacing: Kirigami.Units.smallSpacing

            delegate: Controls.ItemDelegate {
                required property string title
                required property string subtitle
                objectName: "spikeListItem"
                width: ListView.view.width

                contentItem: ColumnLayout {
                    spacing: Kirigami.Units.smallSpacing
                    Kirigami.Heading {
                        level: 4
                        text: title
                        Layout.fillWidth: true
                    }
                    Controls.Label {
                        text: subtitle
                        opacity: 0.75
                        Layout.fillWidth: true
                    }
                }
            }
        }
    }

    Component.onCompleted: {
        Qt.callLater(function() {
            if (!root.narrowMode && pageStack.depth === 1) {
                root.pushFromComponent(backupDetailPage, {});
            }
        });
    }

    Component {
        id: backupsListComponent
        Kirigami.ScrollablePage {
            title: i18n("Backups")

            ListView {
                objectName: "spikeBackupList"
                model: spikeModel
                spacing: Kirigami.Units.smallSpacing

                delegate: Controls.ItemDelegate {
                    required property string title
                    required property string subtitle
                    width: ListView.view.width

                    contentItem: ColumnLayout {
                        spacing: Kirigami.Units.smallSpacing
                        Kirigami.Heading {
                            level: 4
                            text: title
                            Layout.fillWidth: true
                        }
                        Controls.Label {
                            text: subtitle
                            opacity: 0.75
                            Layout.fillWidth: true
                        }
                    }
                }
            }
        }
    }

    Component {
        id: backupDetailPage
        Kirigami.ScrollablePage {
            title: i18n("Personal folder")

            actions: [
                Kirigami.Action {
                    id: backupNowAction
                    objectName: "backupNowAction"
                    icon.name: "media-playback-start"
                    text: sharedTexts.label("back_up_now")
                    tooltip: i18n("Back Up Now (Ctrl+Shift+B)")
                    onTriggered: root.triggerBackupNow()
                },
                Kirigami.Action {
                    icon.name: "configure"
                    text: i18n("Retention…")
                    onTriggered: root.pushFromComponent(retentionPage, {})
                }
            ]

            Kirigami.FormLayout {
                Controls.Label {
                    Kirigami.FormData.label: i18n("Destination:")
                    text: i18n("NAS (REST server)")
                }
                Controls.Label {
                    Kirigami.FormData.label: i18n("Schedule:")
                    text: i18n("Daily 12:00 · AC power and network")
                }
                Controls.Label {
                    Kirigami.FormData.label: i18n("Last run:")
                    text: i18n("Today 12:00, ok, 312 MB new")
                }
                Controls.Button {
                    text: i18n("Configure Retention…")
                    onClicked: root.pushFromComponent(retentionPage, {})
                }
            }
        }
    }

    Component {
        id: retentionPage
        Kirigami.ScrollablePage {
            title: i18n("Retention")

            Kirigami.FormLayout {
                Controls.Label {
                    Kirigami.FormData.label: i18n("Preset:")
                    text: i18n("Standard (7 daily, 4 weekly, 12 monthly)")
                }
                Controls.Label {
                    Kirigami.FormData.label: i18n("Status:")
                    text: i18n("Triggered count: %1", root.backupNowTriggerCount)
                }
            }
        }
    }

    Component {
        id: overviewPage
        Kirigami.ScrollablePage {
            title: i18n("Overview")
            Kirigami.PlaceholderMessage {
                anchors.centerIn: parent
                width: parent.width - Kirigami.Units.largeSpacing * 4
                icon.name: "security-high"
                text: sharedTexts.label("all_data_backed_up")
            }
        }
    }

    Component {
        id: placeholderTabPage
        Kirigami.ScrollablePage {
            property string pageTitle: ""
            title: pageTitle
            Kirigami.PlaceholderMessage {
                anchors.centerIn: parent
                width: parent.width - Kirigami.Units.largeSpacing * 4
                text: pageTitle
            }
        }
    }
}
