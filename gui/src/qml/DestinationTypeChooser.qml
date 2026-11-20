// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// Wizard step: local folder or REST server.
ColumnLayout {
    id: chooser

    required property var draft
    // Prefix of the object names (`<prefix>LocalRadio`, `<prefix>RestRadio`).
    required property string namePrefix

    spacing: Kirigami.Units.mediumSpacing

    function choose(kind) {
        chooser.draft.kind = kind;
        chooser.draft.connectionVerified = false;
    }

    Kirigami.Heading {
        level: 2
        text: i18n("Where should encrypted backups be stored?")
    }

    Controls.RadioButton {
        objectName: chooser.namePrefix + "LocalRadio"
        text: i18n("Local Folder or External Drive")
        checked: chooser.draft.kind === "local"
        Accessible.name: text
        onToggled: if (checked) chooser.choose("local")
    }

    Controls.Label {
        text: i18n("Store snapshots in a local folder or on a connected drive.")
        opacity: 0.75
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.fillWidth: true
        wrapMode: Text.WordWrap
    }

    Controls.RadioButton {
        objectName: chooser.namePrefix + "RestRadio"
        text: Texts.label("rest_server")
        checked: chooser.draft.kind === "rest"
        Accessible.name: text
        onToggled: if (checked) chooser.choose("rest")
    }

    Controls.Label {
        text: i18n("Connect to a restic REST server over HTTPS or HTTP.")
        opacity: 0.75
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.fillWidth: true
        wrapMode: Text.WordWrap
    }
}
