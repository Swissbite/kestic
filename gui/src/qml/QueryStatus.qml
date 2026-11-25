// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// The states of a snapshot query over its list (declare it as a child of the ListView): while
// it runs, a loading placeholder with «Cancel» (without «Cancel» when it cannot be cancelled),
// after a cancel «… cancelled» with «Retry», after an error the error with «Retry», and «Too
// many entries» instead of a list too large to hold. It polls the running query.
// The object names of its parts start with `namePrefix` («search» gives «searchBusy»,
// «searchCancelButton», «searchCancelledMessage», «searchErrorMessage»,
// «searchTooLargeMessage»).
Item {
    id: status

    // The query model: `state`, `error_text`, `too_large`, `too_large_text`, `poll()`.
    required property var queryModel
    required property string namePrefix
    // Accessible name of the busy indicator; empty shows a loading placeholder without «Cancel».
    property string busyText: ""
    // Text shown after the user cancelled the query.
    property string cancelledText: ""

    // «Cancel» while the query runs.
    signal cancelRequested()
    // «Retry» after a cancel or an error.
    signal retryRequested()

    anchors.fill: parent

    Timer {
        interval: 150
        repeat: true
        running: status.queryModel.state === "running"
        onTriggered: status.queryModel.poll()
    }

    ColumnLayout {
        objectName: status.namePrefix + "Busy"
        anchors.centerIn: parent
        width: parent.width - Kirigami.Units.largeSpacing * 4
        visible: status.busyText !== "" && status.queryModel.state === "running"
        spacing: Kirigami.Units.largeSpacing

        Kirigami.LoadingPlaceholder {
            Layout.fillWidth: true
            text: status.busyText
        }
        Controls.Button {
            objectName: status.namePrefix + "CancelButton"
            Layout.alignment: Qt.AlignHCenter
            text: Texts.label("cancel")
            icon.name: "process-stop"
            onClicked: status.cancelRequested()
        }
    }

    Kirigami.LoadingPlaceholder {
        anchors.centerIn: parent
        visible: status.busyText === "" && status.queryModel.state === "running"
    }

    Kirigami.PlaceholderMessage {
        objectName: status.namePrefix + "CancelledMessage"
        anchors.centerIn: parent
        width: parent.width - Kirigami.Units.largeSpacing * 4
        visible: status.queryModel.state === "cancelled" && status.cancelledText !== ""
        icon.name: "process-stop"
        text: status.cancelledText
        helpfulAction: Kirigami.Action {
            text: Texts.label("retry")
            onTriggered: status.retryRequested()
        }
    }

    Kirigami.InlineMessage {
        objectName: status.namePrefix + "ErrorMessage"
        anchors.top: parent.top
        width: parent.width
        type: Kirigami.MessageType.Error
        visible: status.queryModel.state === "error"
        text: status.queryModel.error_text
        actions: [
            Kirigami.Action {
                text: Texts.label("retry")
                onTriggered: status.retryRequested()
            }
        ]
    }

    Kirigami.PlaceholderMessage {
        objectName: status.namePrefix + "TooLargeMessage"
        anchors.centerIn: parent
        width: parent.width - Kirigami.Units.largeSpacing * 4
        visible: status.queryModel.state === "done" && status.queryModel.too_large
        icon.name: "dialog-information"
        text: status.queryModel.too_large_text
    }
}
