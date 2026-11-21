// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// Base of the wizard pages: asks before leaving when something is entered, and has the
// footer with Cancel and Back. A wizard sets `hasInput`, `canGoBack`, the discard dialog
// texts and puts its own buttons into `footerButtons`.
Kirigami.ScrollablePage {
    id: wizard

    required property var rootWindow

    // Something is entered that closing the wizard would lose.
    property bool hasInput: false
    property bool canGoBack: false
    property string discardTitle: ""
    property string discardObjectName: ""
    property alias footerButtons: footerRow.data

    signal backClicked()

    onBackRequested: function(event) {
        if (hasInput) {
            event.accepted = true;
            discardDialog.open();
        }
    }

    function handleEscape() {
        if (rootWindow.closeIfOpen(discardDialog)) return true;
        if (hasInput) {
            discardDialog.open();
            return true;
        }
        return false;
    }

    Kirigami.PromptDialog {
        id: discardDialog
        objectName: wizard.discardObjectName
        title: wizard.discardTitle
        subtitle: Texts.label("nothing_created_yet")
        standardButtons: Kirigami.Dialog.Discard | Kirigami.Dialog.Cancel
        onDiscarded: wizard.rootWindow.closeWizardLayer()
    }

    footer: Controls.ToolBar {
        contentItem: RowLayout {
            id: footerRow
            spacing: Kirigami.Units.smallSpacing

            Controls.Button {
                text: Texts.label("cancel")
                Accessible.name: text
                onClicked: {
                    if (wizard.hasInput) {
                        discardDialog.open();
                    } else {
                        wizard.rootWindow.closeWizardLayer();
                    }
                }
            }

            Item { Layout.fillWidth: true }

            Controls.Button {
                text: Texts.label("back")
                visible: wizard.canGoBack
                Accessible.name: text
                onClicked: wizard.backClicked()
            }
        }
    }
}
