// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kirigami.delegates as KD
import net.daester.kestic 1.0

// Name, location, repository mode, credentials and connection test of a destination.
// Used by both wizards (new destination) and by the edit page (`editing: true`).
ColumnLayout {
    id: form

    required property var draft
    // Prefix of the object names (`<prefix>NameField`, `<prefix>LocationField`, …).
    required property string namePrefix
    // Edit page: no required-field stars, empty passwords keep the saved ones.
    property bool editing: false
    // Order of the repository modes in the combo box.
    property var setupModes: ["init", "connect"]

    readonly property var appBackend: draft.appBackend

    spacing: Kirigami.Units.mediumSpacing

    // The service of a generic destination, from the scheme of its repository URL.
    function serviceLabel() {
        switch (form.draft.location.split(":")[0]) {
        case "b2": return Texts.label("backblaze_b2");
        case "azure": return Texts.label("azure");
        case "gs": return Texts.label("google_cloud");
        case "swift": return Texts.label("swift");
        }
        return Texts.label("rclone");
    }

    function locationEdited() {
        form.draft.connectionVerified = false;
    }

    FolderDialog {
        id: folderDialog
        title: i18n("Choose Repository Folder")
        onAccepted: {
            var reply = {};
            try {
                reply = JSON.parse(form.appBackend.localFolderFromUrl(selectedFolder.toString()));
            } catch (e) {
                reply = {};
            }
            if (reply.error) {
                form.draft.showError(reply.error);
            } else if (reply.path) {
                form.draft.location = reply.path;
                form.locationEdited();
            }
        }
    }

    Kirigami.PromptDialog {
        id: hostKeyDialog
        objectName: form.namePrefix + "HostKeyDialog"
        title: i18n("Trust This Server?")
        standardButtons: Kirigami.Dialog.Cancel
        customFooterActions: [
            Kirigami.Action {
                objectName: form.namePrefix + "TrustHostKeyAction"
                icon.name: "security-high"
                text: i18n("Trust Server")
                onTriggered: {
                    hostKeyDialog.close();
                    form.draft.trustHostKey();
                }
            }
        ]

        ColumnLayout {
            implicitWidth: Kirigami.Units.gridUnit * 24
            spacing: Kirigami.Units.mediumSpacing

            Kirigami.FormLayout {
                Layout.fillWidth: true

                Controls.Label {
                    objectName: form.namePrefix + "HostKeyServer"
                    Kirigami.FormData.label: i18n("Server:")
                    text: form.draft.hostKey ? form.draft.hostKey.host + (form.draft.hostKey.port === 22 ? "" : ":" + form.draft.hostKey.port) : ""
                }

                Controls.Label {
                    objectName: form.namePrefix + "HostKeyType"
                    Kirigami.FormData.label: i18n("Key type:")
                    text: form.draft.hostKey ? form.draft.hostKey.key_type : ""
                }

                Controls.Label {
                    objectName: form.namePrefix + "HostKeyFingerprint"
                    Kirigami.FormData.label: i18n("Fingerprint:")
                    text: form.draft.hostKey ? form.draft.hostKey.fingerprint_sha256 : ""
                    font: Kirigami.Theme.fixedWidthFont
                    textFormat: Text.PlainText
                    wrapMode: Text.WrapAnywhere
                    Layout.fillWidth: true
                }
            }

            Controls.Label {
                text: i18n("Compare the fingerprint with the one of the server. Kestic then remembers the server.")
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
        }
    }

    Connections {
        target: form.draft
        function onHostKeyOffered() {
            hostKeyDialog.open();
        }
    }

    Kirigami.Heading {
        visible: !form.editing
        level: 2
        text: form.draft.kindIsS3
            ? i18n("Which S3-compatible storage should be used?")
            : form.draft.kindIsSftp ? i18n("Which SSH server should be used?")
            : form.draft.asksRcloneRemote ? i18n("Which rclone remote should be used?")
            : form.draft.kindIsGeneric ? i18n("Where on %1 is the repository?", form.serviceLabel())
            : form.draft.kindIsRest ? i18n("Where is the REST server located?") : i18n("Where should the local repository be stored?")
    }

    Kirigami.InlineMessage {
        objectName: form.namePrefix + "FeedbackBanner"
        Layout.fillWidth: true
        // An unknown server is a question, not an error; a changed host key stays an error.
        type: form.draft.testFeedbackOk ? Kirigami.MessageType.Positive
            : form.draft.hostKey !== null ? Kirigami.MessageType.Information
            : Kirigami.MessageType.Error
        visible: form.draft.testFeedback !== ""
        text: form.draft.testFeedback
        actions: [
            Kirigami.Action {
                objectName: form.namePrefix + "ShowHostKeyAction"
                icon.name: "security-high"
                text: i18n("Trust Server…")
                visible: !form.draft.testFeedbackOk && form.draft.hostKey !== null
                onTriggered: hostKeyDialog.open()
            },
            Kirigami.Action {
                objectName: form.namePrefix + "SwitchToInitAction"
                icon.name: "folder-new"
                text: i18n("Create New Repository")
                visible: form.editing && !form.draft.testFeedbackOk && form.draft.setupMode === "connect"
                    && form.appBackend.connection_test_error_kind === "repository_missing"
                onTriggered: {
                    form.draft.setupMode = "init";
                    form.locationEdited();
                    form.draft.runConnectionTest();
                }
            },
            Kirigami.Action {
                objectName: form.namePrefix + "CheckKeysAction"
                icon.name: "dialog-password"
                text: i18n("Check Keys")
                visible: form.draft.kindIsS3 && !form.draft.testFeedbackOk
                    && form.appBackend.connection_test_error_kind === "auth_failed"
                onTriggered: {
                    secretKeyField.forceActiveFocus();
                    secretKeyField.selectAll();
                }
            }
        ]
    }

    Kirigami.InlineMessage {
        objectName: form.namePrefix + "NoteBanner"
        Layout.fillWidth: true
        type: Kirigami.MessageType.Warning
        visible: form.draft.connectionVerified && form.draft.testNote !== ""
        text: form.draft.testNote
    }

    Controls.CheckBox {
        objectName: form.namePrefix + "NoteConfirmCheck"
        visible: form.draft.connectionVerified && form.draft.testNote !== ""
        text: i18n("Use this folder anyway")
        checked: form.draft.testNoteConfirmed
        Accessible.name: text
        onToggled: form.draft.testNoteConfirmed = checked
    }

    // Destination type «External Drive / USB»: the mounted removable drives; choosing one puts
    // its mount point into the path.
    ColumnLayout {
        id: drivePicker

        // `RemovableDriveDto`s from `removableDrivesJson()`.
        property var drives: []
        property string loadError: ""

        function reload() {
            var reply = {};
            try {
                reply = JSON.parse(form.appBackend.removableDrivesJson());
            } catch (e) {
                reply = {};
            }
            drivePicker.drives = reply.drives || [];
            drivePicker.loadError = reply.error || "";
        }

        objectName: form.namePrefix + "DrivePicker"
        visible: !form.editing && form.draft.typeCard === "drive"
        Layout.fillWidth: true
        spacing: Kirigami.Units.smallSpacing
        onVisibleChanged: if (visible) reload()

        Kirigami.ListSectionHeader {
            Layout.fillWidth: true
            text: i18n("Removable Drives")
        }

        Repeater {
            model: drivePicker.drives

            delegate: KD.SubtitleDelegate {
                required property var modelData
                required property int index

                objectName: form.namePrefix + "Drive_" + index
                Layout.fillWidth: true
                // A filesystem without label is named after its mount point.
                text: modelData.label !== "" ? modelData.label : modelData.mount_point.split("/").pop()
                subtitle: modelData.mount_point
                icon.name: "drive-removable-media"
                checkable: true
                checked: form.draft.location.trim() === modelData.mount_point
                highlighted: checked
                Accessible.name: text
                Accessible.description: subtitle
                onClicked: {
                    form.draft.location = modelData.mount_point;
                    form.locationEdited();
                }
            }
        }

        Kirigami.InlineMessage {
            objectName: form.namePrefix + "NoDriveMessage"
            Layout.fillWidth: true
            type: drivePicker.loadError !== "" ? Kirigami.MessageType.Error : Kirigami.MessageType.Information
            visible: drivePicker.drives.length === 0
            text: drivePicker.loadError !== "" ? drivePicker.loadError : i18n("No removable drive is plugged in.")
        }

        Controls.Button {
            objectName: form.namePrefix + "DrivesCheckAgainButton"
            icon.name: "view-refresh"
            text: Texts.label("check_again")
            Accessible.name: text
            onClicked: drivePicker.reload()
        }
    }

    Kirigami.FormLayout {
        Layout.fillWidth: true

        Controls.TextField {
            objectName: form.namePrefix + "NameField"
            Kirigami.FormData.label: form.editing ? Texts.label("name_label") : Texts.label("required_name_label")
            text: form.draft.name
            placeholderText: form.editing ? "" : ((form.draft.kindIsS3 || form.draft.kindIsGeneric) ? i18n("Cloud Backup") : (form.draft.kindIsRest || form.draft.kindIsSftp) ? i18n("NAS") : i18n("Local Backup"))
            onTextChanged: form.draft.name = text
            Accessible.name: i18n("Destination name")
        }

        Controls.Label {
            objectName: form.namePrefix + "NameError"
            visible: !form.draft.nameValid
            text: i18n("Name is required.")
            color: Kirigami.Theme.negativeTextColor
        }

        RowLayout {
            visible: !form.draft.kindIsS3 && !form.draft.kindIsGeneric && !form.draft.kindIsSftp
            Kirigami.FormData.label: form.draft.kindIsRest
                ? (form.editing ? i18n("Address:") : i18n("Address*:"))
                : (form.editing ? i18n("Path:") : i18n("Path*:"))
            Layout.fillWidth: true

            Controls.TextField {
                objectName: form.namePrefix + "LocationField"
                text: form.draft.location
                onTextChanged: {
                    form.draft.location = text;
                    form.locationEdited();
                }
                placeholderText: form.draft.kindIsRest ? "https://backup.example.org:8000/" : "/mnt/backups"
                Accessible.name: form.draft.kindIsRest ? i18n("REST server address") : i18n("Repository path")
                Layout.fillWidth: true
            }

            Controls.Button {
                visible: !form.draft.kindIsRest
                text: Texts.label("choose")
                Accessible.name: i18n("Choose repository folder")
                onClicked: folderDialog.open()
            }
        }

        Controls.Label {
            objectName: form.namePrefix + "LocationError"
            visible: !form.draft.kindIsS3 && !form.draft.kindIsGeneric && !form.draft.kindIsSftp
                && !form.draft.locationValid
            text: form.draft.kindIsRest
                ? i18n("Enter a valid http:// or https:// address.")
                : i18n("Path is required.")
            color: Kirigami.Theme.negativeTextColor
        }

        // SFTP: server, port, user and path; the login uses an SSH key or the ssh-agent.
        Controls.TextField {
            objectName: form.namePrefix + "HostField"
            visible: form.draft.kindIsSftp
            Kirigami.FormData.label: form.editing ? i18n("Host:") : i18n("Host*:")
            text: form.draft.host
            placeholderText: "192.168.1.10"
            onTextChanged: {
                form.draft.host = text;
                form.locationEdited();
            }
            Accessible.name: i18n("SSH server")
            Layout.fillWidth: true
        }

        Controls.Label {
            objectName: form.namePrefix + "HostError"
            visible: form.draft.kindIsSftp && !form.draft.hostValid
            text: i18n("Enter a host name or an IP address.")
            color: Kirigami.Theme.negativeTextColor
        }

        Controls.TextField {
            objectName: form.namePrefix + "PortField"
            visible: form.draft.kindIsSftp
            Kirigami.FormData.label: i18n("Port:")
            text: form.draft.port
            inputMethodHints: Qt.ImhDigitsOnly
            onTextChanged: {
                form.draft.port = text;
                form.locationEdited();
            }
            Accessible.name: i18n("SSH port")
        }

        Controls.Label {
            objectName: form.namePrefix + "PortError"
            visible: form.draft.kindIsSftp && !form.draft.portValid
            text: i18n("Enter a port from 1 to 65535.")
            color: Kirigami.Theme.negativeTextColor
        }

        Controls.TextField {
            objectName: form.namePrefix + "SftpUserField"
            visible: form.draft.kindIsSftp
            Kirigami.FormData.label: form.editing ? i18n("User:") : i18n("User*:")
            text: form.draft.username
            onTextChanged: {
                form.draft.username = text;
                form.locationEdited();
            }
            Accessible.name: i18n("SSH user")
            Layout.fillWidth: true
        }

        Controls.Label {
            objectName: form.namePrefix + "SftpUserError"
            visible: form.draft.kindIsSftp && !form.draft.sftpUserValid
            text: i18n("Enter the user name on the server, without spaces or '@'.")
            color: Kirigami.Theme.negativeTextColor
        }

        Controls.TextField {
            objectName: form.namePrefix + "SftpPathField"
            visible: form.draft.kindIsSftp
            Kirigami.FormData.label: form.editing ? i18n("Path:") : i18n("Path*:")
            text: form.draft.sftpPath
            placeholderText: "/srv/backup"
            onTextChanged: {
                form.draft.sftpPath = text;
                form.locationEdited();
            }
            Accessible.name: i18n("Repository path on the server")
            Layout.fillWidth: true
        }

        Controls.Label {
            objectName: form.namePrefix + "SftpPathError"
            visible: form.draft.kindIsSftp && !form.draft.sftpPathValid
            text: i18n("Enter an absolute path or one relative to the home folder, not starting with '~'.")
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.Wrap
            Layout.fillWidth: true
            Layout.preferredWidth: Kirigami.Units.gridUnit * 24
        }

        Controls.Label {
            objectName: form.namePrefix + "SshKeyHint"
            visible: form.draft.kindIsSftp
            Kirigami.FormData.label: i18n("Login:")
            Kirigami.FormData.labelAlignment: Qt.AlignTop
            text: i18n("With your SSH key or the ssh-agent. Copy your public key to the server first, for example with ssh-copy-id.")
            wrapMode: Text.Wrap
            Layout.fillWidth: true
            Layout.preferredWidth: Kirigami.Units.gridUnit * 24
        }

        // Generic form: the repository URL, or for the rclone card only the remote after `rclone:`.
        Controls.TextField {
            objectName: form.namePrefix + "UrlField"
            visible: form.draft.kindIsGeneric
            Kirigami.FormData.label: form.draft.asksRcloneRemote
                ? (form.editing ? i18n("rclone remote:") : i18n("rclone remote*:"))
                : (form.editing ? i18n("Repository URL:") : i18n("Repository URL*:"))
            text: form.draft.asksRcloneRemote ? form.draft.location.replace(/^rclone:/, "") : form.draft.location
            placeholderText: form.draft.asksRcloneRemote ? "myremote:backups/laptop" : "b2:bucket-name:backups/laptop"
            onTextChanged: {
                var url = form.draft.asksRcloneRemote ? "rclone:" + text.replace(/^rclone:/, "") : text;
                if (url !== form.draft.location) {
                    form.draft.location = url;
                    form.locationEdited();
                }
            }
            Accessible.name: form.draft.asksRcloneRemote ? i18n("rclone remote") : i18n("Repository URL")
            Layout.fillWidth: true
        }

        Controls.Label {
            objectName: form.namePrefix + "GenericError"
            visible: form.draft.kindIsGeneric && form.draft.genericErrors.url !== ""
            text: form.draft.genericErrors.url
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.Wrap
            Layout.fillWidth: true
            Layout.preferredWidth: Kirigami.Units.gridUnit * 24
        }

        Kirigami.Separator {
            visible: form.draft.kindIsGeneric
            Kirigami.FormData.isSection: true
            Kirigami.FormData.label: i18n("Environment Variables")
        }

        Controls.Label {
            objectName: form.namePrefix + "EnvHint"
            visible: form.draft.kindIsGeneric
            text: i18n("Settings the storage service reads, for example its account ID and key. Secret values are kept only in the password store.")
            wrapMode: Text.Wrap
            Layout.fillWidth: true
            Layout.preferredWidth: Kirigami.Units.gridUnit * 24
        }

        // The rows in one layout: the form layout keeps one item while rows come and go.
        ColumnLayout {
            visible: form.draft.kindIsGeneric && form.draft.envModel.count > 0
            Layout.fillWidth: true

            // Column labels; the switch and the button stand in invisibly so the labels sit over
            // their fields.
            RowLayout {
                Controls.Label {
                    text: i18n("Name")
                    font: Kirigami.Theme.smallFont
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                    Layout.fillWidth: true
                }

                Controls.Label {
                    text: i18n("Value")
                    font: Kirigami.Theme.smallFont
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                    Layout.fillWidth: true
                }

                Controls.Switch {
                    text: i18n("Secret")
                    opacity: 0
                    enabled: false
                    activeFocusOnTab: false
                    Accessible.ignored: true
                }

                Controls.ToolButton {
                    icon.name: "list-remove"
                    display: Controls.AbstractButton.IconOnly
                    opacity: 0
                    enabled: false
                    activeFocusOnTab: false
                    Accessible.ignored: true
                }
            }

            Repeater {
                model: form.draft.envModel

                delegate: RowLayout {
                    id: envRow

                    required property int index
                    required property string name
                    required property bool secret
                    required property string value

                    objectName: form.namePrefix + "EnvRow_" + index
                    Layout.fillWidth: true

                    Controls.TextField {
                        objectName: form.namePrefix + "EnvName_" + envRow.index
                        text: envRow.name
                        placeholderText: i18n("NAME")
                        Accessible.name: i18n("Variable name")
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                        Layout.fillWidth: true
                        onTextChanged: if (text !== envRow.name) form.draft.editEnvVar(envRow.index, "name", text)
                    }

                    Controls.TextField {
                        objectName: form.namePrefix + "EnvValue_" + envRow.index
                        text: envRow.value
                        echoMode: envRow.secret ? TextInput.Password : TextInput.Normal
                        placeholderText: form.editing && envRow.secret ? i18n("Leave empty to keep the saved value") : i18n("Value")
                        Accessible.name: i18n("Variable value")
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                        Layout.fillWidth: true
                        onTextChanged: if (text !== envRow.value) form.draft.editEnvVar(envRow.index, "value", text)
                    }

                    Controls.Switch {
                        objectName: form.namePrefix + "EnvSecret_" + envRow.index
                        text: i18n("Secret")
                        checked: envRow.secret
                        Accessible.name: i18n("Secret: keep the value only in the password store")
                        onToggled: form.draft.editEnvVar(envRow.index, "secret", checked)
                    }

                    Controls.ToolButton {
                        objectName: form.namePrefix + "EnvRemove_" + envRow.index
                        icon.name: "list-remove"
                        text: i18n("Remove Variable")
                        display: Controls.AbstractButton.IconOnly
                        Accessible.name: text
                        Controls.ToolTip.text: text
                        Controls.ToolTip.visible: hovered
                        onClicked: form.draft.removeEnvVar(envRow.index)
                    }
                }
            }
        }

        Controls.Label {
            objectName: form.namePrefix + "EnvError"
            visible: form.draft.kindIsGeneric && form.draft.genericErrors.env !== ""
            text: form.draft.genericErrors.env
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.Wrap
            Layout.fillWidth: true
            Layout.preferredWidth: Kirigami.Units.gridUnit * 24
        }

        Controls.Button {
            objectName: form.namePrefix + "EnvAdd"
            visible: form.draft.kindIsGeneric
            icon.name: "list-add"
            text: i18n("Add Variable")
            Accessible.name: text
            onClicked: form.draft.addEnvVar()
        }

        Controls.Label {
            objectName: form.namePrefix + "EnvSecretsError"
            visible: form.draft.kindIsGeneric && !form.draft.keysValid
            text: i18n("Enter a value for every secret variable.")
            color: Kirigami.Theme.negativeTextColor
        }

        Controls.TextField {
            objectName: form.namePrefix + "EndpointField"
            visible: form.draft.kindIsS3
            Kirigami.FormData.label: form.editing ? i18n("Endpoint:") : i18n("Endpoint*:")
            text: form.draft.endpoint
            placeholderText: "https://s3.example.com"
            onTextChanged: {
                form.draft.endpoint = text;
                form.locationEdited();
            }
            Accessible.name: i18n("S3 endpoint")
            Layout.fillWidth: true
        }

        Controls.Label {
            objectName: form.namePrefix + "EndpointError"
            visible: form.draft.kindIsS3 && !form.draft.endpointValid
            text: i18n("Enter the address with http:// or https://, without a path.")
            color: Kirigami.Theme.negativeTextColor
        }

        Controls.TextField {
            objectName: form.namePrefix + "BucketField"
            visible: form.draft.kindIsS3
            Kirigami.FormData.label: form.editing ? i18n("Bucket:") : i18n("Bucket*:")
            text: form.draft.bucket
            onTextChanged: {
                form.draft.bucket = text;
                form.locationEdited();
            }
            Accessible.name: i18n("S3 bucket")
            Layout.fillWidth: true
        }

        Controls.Label {
            objectName: form.namePrefix + "BucketError"
            visible: form.draft.kindIsS3 && !form.draft.bucketValid
            text: i18n("Use 3 to 63 lowercase letters, digits, dots or hyphens.")
            color: Kirigami.Theme.negativeTextColor
        }

        Controls.TextField {
            objectName: form.namePrefix + "S3PathField"
            visible: form.draft.kindIsS3
            Kirigami.FormData.label: i18n("Path:")
            text: form.draft.s3Path
            onTextChanged: {
                form.draft.s3Path = text;
                form.locationEdited();
            }
            Accessible.name: i18n("Path in the bucket")
            Layout.fillWidth: true
        }

        Controls.Label {
            objectName: form.namePrefix + "S3PathError"
            visible: form.draft.kindIsS3 && !form.draft.s3PathValid
            text: i18n("Use no spaces, quotes, #, ? or %.")
            color: Kirigami.Theme.negativeTextColor
        }

        Controls.TextField {
            objectName: form.namePrefix + "RegionField"
            visible: form.draft.kindIsS3
            Kirigami.FormData.label: i18n("Region:")
            text: form.draft.region
            onTextChanged: {
                form.draft.region = text;
                form.locationEdited();
            }
            Accessible.name: i18n("S3 region")
            Layout.fillWidth: true
        }

        Controls.TextField {
            objectName: form.namePrefix + "AccessKeyField"
            visible: form.draft.kindIsS3
            Kirigami.FormData.label: form.editing ? i18n("Access key:") : i18n("Access key*:")
            placeholderText: form.editing ? i18n("Leave empty to keep the saved key") : ""
            text: form.draft.accessKeyId
            onTextChanged: {
                form.draft.accessKeyId = text;
                form.locationEdited();
            }
            Accessible.name: i18n("S3 access key")
            Layout.fillWidth: true
        }

        Controls.TextField {
            id: secretKeyField
            objectName: form.namePrefix + "SecretKeyField"
            visible: form.draft.kindIsS3
            Kirigami.FormData.label: form.editing ? i18n("Secret key:") : i18n("Secret key*:")
            echoMode: TextInput.Password
            placeholderText: form.editing ? i18n("Leave empty to keep the saved key") : ""
            text: form.draft.secretAccessKey
            onTextChanged: {
                form.draft.secretAccessKey = text;
                form.locationEdited();
            }
            Accessible.name: i18n("S3 secret key")
            Layout.fillWidth: true
        }

        Controls.Label {
            objectName: form.namePrefix + "KeysError"
            visible: form.draft.kindIsS3 && !form.draft.keysValid
            text: i18n("Enter the access key and the secret key.")
            color: Kirigami.Theme.negativeTextColor
        }

        // Ends the variables section of the generic form, so the repository rows stand apart.
        Kirigami.Separator {
            visible: form.draft.kindIsGeneric
            Kirigami.FormData.isSection: true
        }

        Controls.ComboBox {
            objectName: form.namePrefix + "SetupModeCombo"
            Kirigami.FormData.label: i18n("Repository:")
            model: form.setupModes.map(mode => mode === "init" ? i18n("Create new repository") : i18n("Connect existing repository"))
            currentIndex: Math.max(0, form.setupModes.indexOf(form.draft.setupMode))
            Accessible.name: i18n("Repository setup mode")
            onActivated: {
                form.draft.setupMode = form.setupModes[currentIndex];
                form.locationEdited();
            }
        }

        Controls.TextField {
            visible: form.draft.kindIsRest
            Kirigami.FormData.label: Texts.label("username_label")
            text: form.draft.username
            onTextChanged: {
                form.draft.username = text;
                form.locationEdited();
            }
            Accessible.name: i18n("REST username")
        }

        Controls.TextField {
            visible: form.draft.kindIsRest
            Kirigami.FormData.label: i18n("Server password:")
            echoMode: TextInput.Password
            placeholderText: form.editing ? i18n("Leave empty to keep the saved password") : ""
            text: form.draft.restPassword
            onTextChanged: {
                form.draft.restPassword = text;
                form.locationEdited();
            }
            Accessible.name: i18n("REST server password")
        }

        Controls.TextField {
            visible: form.editing || form.draft.setupMode === "connect"
            Kirigami.FormData.label: form.editing ? i18n("Repository password:") : i18n("Repository password*:")
            echoMode: TextInput.Password
            placeholderText: form.editing ? i18n("Leave empty to keep the saved password") : ""
            text: form.draft.repoPassword
            onTextChanged: {
                if (visible && form.draft.repoPassword !== text) {
                    form.draft.repoPassword = text;
                    form.locationEdited();
                }
            }
            Accessible.name: form.editing ? Texts.label("repository_password") : i18n("Existing repository password")
        }

        Controls.Button {
            objectName: form.namePrefix + "TestButton"
            icon.name: "network-connect"
            text: i18n("Test Connection")
            enabled: form.draft.detailsValid && form.appBackend.blocking_kind === ""
            Accessible.name: text
            onClicked: form.draft.runConnectionTest()
        }
    }
}
