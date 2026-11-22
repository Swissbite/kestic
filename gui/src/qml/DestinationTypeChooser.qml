// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kirigami.delegates as KD
import net.daester.kestic 1.0

// Wizard step «destination type»: the cards that are ready, then those that need a program to
// be installed. An entry that needs installation opens the package name, a Discover link, the
// install command and «Check Again».
ColumnLayout {
    id: chooser

    required property var draft
    // Prefix of the object names (`<prefix>TypeCard_<card>`, `<prefix>InstallPanel`, …).
    required property string namePrefix

    readonly property var appBackend: draft.appBackend
    // Cards from `destinationTypeCardsJson()`, ready ones first.
    property var cards: []
    property string loadError: ""
    // The card that needs installation and is opened.
    property string openedCard: ""
    property bool commandCopied: false
    // Cards whose details form exists; the other ready cards can not be chosen yet.
    readonly property var cardsWithForm: ["drive", "folder", "rest"]
    readonly property var readyCards: cards.filter(function(c) { return c.ready; })
    readonly property var cardsNeedingInstallation: cards.filter(function(c) { return !c.ready; })

    spacing: Kirigami.Units.mediumSpacing

    function reload() {
        var reply = {};
        try {
            reply = JSON.parse(chooser.appBackend.destinationTypeCardsJson());
        } catch (e) {
            reply = {};
        }
        chooser.cards = reply.cards || [];
        chooser.loadError = reply.error || "";
        chooser.commandCopied = false;
        var stillMissing = chooser.cards.some(function(c) {
            return c.card === chooser.openedCard && !c.ready;
        });
        if (!stillMissing) {
            chooser.openedCard = "";
        }
    }

    function choose(card) {
        chooser.draft.typeCard = card;
        chooser.draft.kind = card === "rest" ? "rest" : "local";
        chooser.draft.connectionVerified = false;
    }

    function cardTitle(card) {
        switch (card) {
        case "drive": return i18n("External Drive / USB");
        case "folder": return i18n("Folder / Other Disk");
        case "sftp": return i18n("NAS / Server via SSH");
        case "rest": return Texts.label("rest_server");
        case "cloud": return i18n("Cloud Storage");
        case "rclone": return i18n("Cloud Services via rclone");
        }
        return card;
    }

    function cardDescription(card) {
        switch (card) {
        case "drive": return i18n("A drive you plug in");
        case "folder": return i18n("Local folder, for example on a second hard disk");
        case "sftp": return i18n("SFTP");
        case "rest": return i18n("For example on the NAS");
        case "cloud": return i18n("S3-compatible, Backblaze B2, Azure, Google Cloud, Swift");
        case "rclone": return i18n("Google Drive, OneDrive, Dropbox …");
        }
        return "";
    }

    function cardIcon(card) {
        switch (card) {
        case "drive": return "drive-removable-media";
        case "folder": return "folder";
        case "sftp": return "network-server";
        case "rest": return "network-server";
        }
        return "folder-cloud";
    }

    Component.onCompleted: if (visible) reload()
    onVisibleChanged: if (visible) reload()

    Kirigami.Heading {
        level: 2
        text: i18n("Where should encrypted backups be stored?")
    }

    Kirigami.InlineMessage {
        objectName: chooser.namePrefix + "TypeLoadError"
        Layout.fillWidth: true
        type: Kirigami.MessageType.Error
        visible: chooser.loadError !== ""
        text: chooser.loadError
        actions: [
            Kirigami.Action {
                text: i18n("Check Again")
                onTriggered: chooser.reload()
            }
        ]
    }

    Kirigami.ListSectionHeader {
        objectName: chooser.namePrefix + "ReadySection"
        Layout.fillWidth: true
        visible: chooser.readyCards.length > 0
        text: i18n("Ready")
    }

    Repeater {
        model: chooser.readyCards

        delegate: KD.SubtitleDelegate {
            required property var modelData
            readonly property bool hasForm: chooser.cardsWithForm.indexOf(modelData.card) >= 0

            objectName: chooser.namePrefix + "TypeCard_" + modelData.card
            Layout.fillWidth: true
            text: chooser.cardTitle(modelData.card)
            subtitle: chooser.cardDescription(modelData.card)
            icon.name: chooser.cardIcon(modelData.card)
            enabled: hasForm
            checkable: true
            checked: chooser.draft.typeCard === modelData.card
            highlighted: checked
            Accessible.name: text
            Accessible.description: subtitle
            onClicked: chooser.choose(modelData.card)
        }
    }

    Kirigami.ListSectionHeader {
        objectName: chooser.namePrefix + "NeedsInstallationSection"
        Layout.fillWidth: true
        visible: chooser.cardsNeedingInstallation.length > 0
        text: i18n("Needs Installation")
    }

    Repeater {
        model: chooser.cardsNeedingInstallation

        delegate: ColumnLayout {
            id: missingEntry

            required property var modelData
            readonly property bool opened: chooser.openedCard === modelData.card

            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            KD.SubtitleDelegate {
                objectName: chooser.namePrefix + "TypeCard_" + missingEntry.modelData.card
                Layout.fillWidth: true
                text: chooser.cardTitle(missingEntry.modelData.card)
                subtitle: chooser.cardDescription(missingEntry.modelData.card) + " · "
                    + i18n("needs the program '%1'", missingEntry.modelData.missing_program)
                icon.name: chooser.cardIcon(missingEntry.modelData.card)
                highlighted: missingEntry.opened
                Accessible.name: text
                Accessible.description: subtitle
                onClicked: {
                    chooser.commandCopied = false;
                    chooser.openedCard = missingEntry.opened ? "" : missingEntry.modelData.card;
                }
            }

            ColumnLayout {
                objectName: chooser.namePrefix + "InstallPanel"
                visible: missingEntry.opened
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.gridUnit * 2
                Layout.rightMargin: Kirigami.Units.largeSpacing
                spacing: Kirigami.Units.smallSpacing

                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: i18n("Install the program '%1', then check again.", missingEntry.modelData.missing_program)
                }

                Controls.Label {
                    objectName: chooser.namePrefix + "InstallPackageLabel"
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: i18n("Package: %1", missingEntry.modelData.package)
                }

                Controls.Button {
                    objectName: chooser.namePrefix + "OpenInDiscoverButton"
                    readonly property string url: missingEntry.modelData.appstream_url
                    text: i18n("Open in Discover")
                    Accessible.name: text
                    onClicked: Qt.openUrlExternally(url)
                }

                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: i18n("Or install it in a terminal:")
                }

                RowLayout {
                    Layout.fillWidth: true

                    Controls.TextField {
                        objectName: chooser.namePrefix + "InstallCommandField"
                        Layout.fillWidth: true
                        readOnly: true
                        font.family: "monospace"
                        text: missingEntry.modelData.install_command
                        Accessible.name: i18n("Install command")
                    }

                    Controls.Button {
                        objectName: chooser.namePrefix + "CopyInstallCommandButton"
                        icon.name: "edit-copy"
                        text: Texts.label("copy")
                        Accessible.name: text
                        onClicked: {
                            chooser.appBackend.copyToClipboard(missingEntry.modelData.install_command);
                            chooser.commandCopied = true;
                        }
                    }
                }

                Controls.Label {
                    visible: chooser.commandCopied
                    Layout.fillWidth: true
                    text: Texts.label("command_copied")
                    opacity: 0.75
                }

                Controls.Button {
                    objectName: chooser.namePrefix + "CheckAgainButton"
                    text: i18n("Check Again")
                    Accessible.name: text
                    onClicked: chooser.reload()
                }
            }
        }
    }
}
