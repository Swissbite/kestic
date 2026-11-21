// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// «About Kestic» as a dialog window of its own, like «Configure Kestic». Escape and [Close]
// close it; it stays alive for the next opening.
Kirigami.ApplicationWindow {
    id: aboutWindow
    objectName: "aboutWindow"
    title: Texts.label("about_kestic")
    flags: Qt.Dialog
    width: Math.round(Kirigami.Units.gridUnit * 34)
    height: Math.round(Kirigami.Units.gridUnit * 32)
    minimumWidth: Math.round(Kirigami.Units.gridUnit * 22)
    minimumHeight: Math.round(Kirigami.Units.gridUnit * 18)
    visible: false

    required property var appBackend

    onClosing: function(close) {
        close.accepted = false;
        visible = false;
    }

    function openWindow() {
        visible = true;
        if (Qt.platform.pluginName !== "offscreen") {
            raise();
        }
        requestActivate();
    }

    Shortcut {
        sequence: "Escape"
        onActivated: aboutWindow.close()
    }

    header: Kirigami.InlineMessage {
        objectName: "buildsDifferMessage"
        position: Kirigami.InlineMessage.Position.Header
        type: Kirigami.MessageType.Information
        visible: BuildInfo.differsFromApp(aboutWindow.appBackend.daemon_version, aboutWindow.appBackend.daemon_build_commit)
        text: i18n("The background service runs another version. Close and reopen Kestic to load the new version.")
    }

    pageStack.globalToolBar.style: Kirigami.ApplicationHeaderStyle.None
    pageStack.initialPage: MainAboutPage {
        appBackend: aboutWindow.appBackend
    }

    footer: Controls.DialogButtonBox {
        Controls.Button {
            objectName: "aboutCloseButton"
            text: Texts.label("close")
            icon.name: "dialog-close"
            Accessible.name: text
            Controls.DialogButtonBox.buttonRole: Controls.DialogButtonBox.RejectRole
        }
        onRejected: aboutWindow.close()
    }
}
