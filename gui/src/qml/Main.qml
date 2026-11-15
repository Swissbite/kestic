// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

Kirigami.ApplicationWindow {
    id: root
    title: "Kestic"
    width: 900
    height: 600
    visible: true

    SpikeListModel {
        id: spikeModel
        objectName: "spikeModel"
    }

    pageStack.initialPage: Kirigami.ScrollablePage {
        title: "Backups"

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
}
