// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import net.daester.kestic 1.0

// Wizard step «destination type»: the cards that are ready, then those that need a program to
// be installed. An entry that needs installation opens the package name, the install command
// and «Check Again».
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
    // Choices under «Cloud Storage»: S3 has its own form, the others the generic form with the
    // scheme filled in.
    readonly property var cloudServices: [
        { service: "s3", label: Texts.label("s3_compatible") },
        { service: "b2", label: Texts.label("backblaze_b2") },
        { service: "azure", label: Texts.label("azure") },
        { service: "gs", label: Texts.label("google_cloud") },
        { service: "swift", label: Texts.label("swift") }
    ]
    readonly property var readyCards: cards.filter(function(c) { return c.ready; })
    readonly property var cardsNeedingInstallation: cards.filter(function(c) { return !c.ready; })
    // The chosen card is shown and ready.
    readonly property bool choiceValid: readyCards.some(function(c) {
        return c.card === chooser.draft.typeCard;
    })

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
        if (card === "rclone") {
            chooseGeneric("rclone");
        } else {
            chooser.draft.kind = card === "rest" ? "rest" : card === "sftp" ? "sftp" : card === "cloud" ? "s3" : "local";
        }
        chooser.draft.connectionVerified = false;
    }

    // The generic form with `scheme` filled into the repository URL (kept if it already has it).
    function chooseGeneric(scheme) {
        chooser.draft.kind = "generic";
        if (chooser.draft.location.indexOf(scheme + ":") !== 0) {
            chooser.draft.location = scheme + ":";
        }
        chooser.draft.connectionVerified = false;
    }

    // The chosen service under «Cloud Storage».
    function cloudService() {
        if (chooser.draft.kind !== "generic") {
            return "s3";
        }
        var colon = chooser.draft.location.indexOf(":");
        return colon > 0 ? chooser.draft.location.substring(0, colon) : "";
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
        case "sftp": return Texts.label("sftp");
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
                text: Texts.label("check_again")
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

    // Not a list: the cards of the ready destination types
    Repeater {
        model: chooser.readyCards

        delegate: TitleSubtitleDelegate {
            required property var modelData

            objectName: chooser.namePrefix + "TypeCard_" + modelData.card
            Layout.fillWidth: true
            text: chooser.cardTitle(modelData.card)
            subtitle: chooser.cardDescription(modelData.card)
            icon.name: chooser.cardIcon(modelData.card)
            checkable: true
            checked: chooser.draft.typeCard === modelData.card
            highlighted: checked
            Accessible.name: text
            Accessible.description: subtitle
            onClicked: chooser.choose(modelData.card)
        }
    }

    // «Cloud Storage»: the kind of cloud storage.
    ColumnLayout {
        objectName: chooser.namePrefix + "CloudServiceChoice"
        visible: chooser.draft.typeCard === "cloud"
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.gridUnit * 2
        spacing: Kirigami.Units.smallSpacing

        // Not a list: radio choices of the cloud service
        Repeater {
            model: chooser.cloudServices

            delegate: Controls.RadioButton {
                required property var modelData

                objectName: chooser.namePrefix + "CloudService_" + modelData.service
                text: modelData.label
                checked: chooser.cloudService() === modelData.service
                Accessible.name: text
                onClicked: {
                    if (modelData.service === "s3") {
                        chooser.draft.kind = "s3";
                        chooser.draft.connectionVerified = false;
                    } else {
                        chooser.chooseGeneric(modelData.service);
                    }
                }
            }
        }
    }

    Kirigami.ListSectionHeader {
        objectName: chooser.namePrefix + "NeedsInstallationSection"
        Layout.fillWidth: true
        visible: chooser.cardsNeedingInstallation.length > 0
        text: i18n("Needs Installation")
    }

    // Not a list: the cards of the destination types that need a program
    Repeater {
        model: chooser.cardsNeedingInstallation

        delegate: ColumnLayout {
            id: missingEntry

            required property var modelData
            readonly property bool opened: chooser.openedCard === modelData.card

            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            TitleSubtitleDelegate {
                objectName: chooser.namePrefix + "TypeCard_" + missingEntry.modelData.card
                Layout.fillWidth: true
                text: chooser.cardTitle(missingEntry.modelData.card)
                subtitle: chooser.cardDescription(missingEntry.modelData.card) + " · "
                    + i18n("needs the program '%1'", missingEntry.modelData.missing_program)
                icon.name: chooser.cardIcon(missingEntry.modelData.card)
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

                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: i18n("Install it in a terminal:")
                }

                RowLayout {
                    Layout.fillWidth: true

                    Controls.TextField {
                        objectName: chooser.namePrefix + "InstallCommandField"
                        Layout.fillWidth: true
                        readOnly: true
                        font: Kirigami.Theme.fixedWidthFont
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
                    text: Texts.label("check_again")
                    Accessible.name: text
                    onClicked: chooser.reload()
                }
            }
        }
    }
}
