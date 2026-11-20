// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.Dialog {
    id: dialog
    objectName: "showCommandDialog"
    title: dialog.cmdTitle !== "" ? i18n("Command · %1", dialog.cmdTitle) : i18n("Command")
    standardButtons: Kirigami.Dialog.Close
    padding: Kirigami.Units.largeSpacing

    required property var rootWindow
    required property var appBackend
    property string cmdText: ""
    property string cmdTitle: ""

    customFooterActions: [
        Kirigami.Action {
            icon.name: "edit-copy"
            text: i18n("Copy")
            onTriggered: {
                dialog.appBackend.copyToClipboard(dialog.cmdText);
                dialog.rootWindow.showPassiveNotification(i18n("Command copied to clipboard"));
            }
        }
    ]

    ColumnLayout {
        implicitWidth: Kirigami.Units.gridUnit * 28
        spacing: Kirigami.Units.smallSpacing

        Controls.Label {
            text: i18n("restic command (without passwords):")
            opacity: 0.8
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        Controls.TextArea {
            id: commandTextArea
            objectName: "showCommandTextArea"
            readOnly: true
            font: Kirigami.Theme.fixedWidthFont
            text: dialog.cmdText
            wrapMode: TextEdit.Wrap
            Layout.fillWidth: true

            Keys.onTabPressed: function(event) {
                var next = nextItemInFocusChain(true);
                if (next) next.forceActiveFocus(Qt.TabFocusReason);
                event.accepted = true;
            }
            Keys.onBacktabPressed: function(event) {
                var prev = nextItemInFocusChain(false);
                if (prev) prev.forceActiveFocus(Qt.BacktabFocusReason);
                event.accepted = true;
            }

            Rectangle {
                anchors.fill: parent
                color: "transparent"
                radius: Kirigami.Units.cornerRadius
                border.width: commandTextArea.activeFocus ? 2 : 0
                border.color: Kirigami.Theme.highlightColor
                visible: border.width > 0
            }
        }
    }
}
