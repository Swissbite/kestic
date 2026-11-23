// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import net.daester.kestic 1.0

// Fields, checks and connection test of a destination that is being added or edited.
QtObject {

    required property var appBackend
    property string destinationId: ""
    property string kind: "local"
    // Card chosen in the step «destination type» (`drive`, `folder`, `rest`, …).
    property string typeCard: "folder"
    property string setupMode: "init"
    property string name: ""
    property string location: ""
    property string username: ""
    property string restPassword: ""
    // S3-compatible storage: endpoint with scheme, bucket, optional path and region, both keys.
    property string endpoint: ""
    property string bucket: ""
    property string s3Path: ""
    property string region: ""
    property string accessKeyId: ""
    property string secretAccessKey: ""
    // Generic form (B2, Azure, Google Cloud, Swift, rclone): the repository URL is `location`;
    // the variables as rows `{name, secret, value, secretTouched}`. A row is changed in place, so
    // the field being typed in keeps its focus; `envRevision` counts the changes for bindings.
    // `secretTouched`: «Geheim» was switched by hand and no longer follows the name.
    property ListModel envModel: ListModel {}
    property int envRevision: 0
    property string repoPassword: ""
    property string repoPasswordConfirm: ""
    property bool connectionVerified: false
    property string testFeedback: ""
    property bool testFeedbackOk: false
    // Warning of the last successful connection test (a folder on a CIFS/SMB share) and whether
    // the user confirmed it.
    property string testNote: ""
    property bool testNoteConfirmed: false

    readonly property bool kindIsRest: kind === "rest"
    readonly property bool kindIsS3: kind === "s3"
    readonly property bool kindIsGeneric: kind === "generic"
    // The rclone card asks for the remote only; the URL is `rclone:` and the remote.
    readonly property bool asksRcloneRemote: kindIsGeneric && typeCard === "rclone"
    // Why `core` refuses the URL and the variables of the generic form (`{url, env}`, each empty
    // when accepted).
    readonly property var genericErrors: kindIsGeneric
        ? JSON.parse(appBackend.genericFieldsErrors(location, JSON.stringify(envFields())))
        : ({ url: "", env: "" })
    readonly property bool nameValid: name.trim().length > 0
    readonly property bool locationValid: kindIsS3
        ? endpointValid && bucketValid && s3PathValid
        : kindIsGeneric
            ? genericErrors.url === "" && genericErrors.env === ""
            : kindIsRest
                ? /^https?:\/\//i.test(location.trim())
                : location.trim().length > 0
    readonly property bool endpointValid: /^https?:\/\/[^\/@]+\/?$/i.test(endpoint.trim())
    // S3 bucket naming rules: 3 to 63 lowercase letters, digits, dots or hyphens, starting and
    // ending with a letter or digit, no two dots in a row.
    readonly property bool bucketValid: /^[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]$/.test(bucket.trim())
        && bucket.indexOf("..") < 0
        && !/^\d+\.\d+\.\d+\.\d+$/.test(bucket.trim())
    // restic reads the S3 address as URL (`#` and `?` cut it, `%` decodes) and repeats it unquoted
    // in the messages Kestic reads, so the path has no such characters, spaces or quotes.
    readonly property bool s3PathValid: !/[\s"#?%\x00-\x1f\x7f]/.test(s3Path.trim())
    // A new S3 destination needs both keys; when editing, empty keys keep the saved ones.
    readonly property bool keysValid: kindIsGeneric
        ? destinationId !== "" || envList().every(function(v) { return !v.secret || v.value.length > 0; })
        : !kindIsS3 || destinationId !== ""
            || (accessKeyId.trim().length > 0 && secretAccessKey.trim().length > 0)
    readonly property bool detailsValid: nameValid && locationValid && keysValid
    // The connection test passed and its warning, if any, is confirmed.
    readonly property bool connectionAccepted: connectionVerified && (testNote === "" || testNoteConfirmed)
    readonly property bool passwordsMatch: repoPassword.length > 0 && (setupMode === "connect" || repoPassword === repoPasswordConfirm)
    readonly property bool hasAnyInput: name.trim().length > 0 || location.trim().length > 0 || repoPassword.length > 0
        || endpoint.trim().length > 0 || bucket.trim().length > 0 || accessKeyId.length > 0 || secretAccessKey.length > 0
        || envModel.count > 0

    // The rows of `envModel` as plain objects; reading `envRevision` makes bindings that call it
    // follow every change of a row.
    function envList() {
        var rows = [];
        if (envRevision < 0) {
            return rows;
        }
        for (var i = 0; i < envModel.count; i++) {
            var row = envModel.get(i);
            rows.push({ name: row.name, secret: row.secret, value: row.value, secretTouched: row.secretTouched });
        }
        return rows;
    }

    // The variables for `fieldsJson()`: secret values stay out (they go in `backendSecretsJson()`).
    function envFields() {
        return envList().map(function(v) {
            return { name: v.name.trim(), secret: v.secret, value: v.secret ? "" : v.value };
        });
    }

    function envChanged() {
        envRevision++;
        connectionVerified = false;
    }

    function addEnvVar() {
        envModel.append({ name: "", secret: false, value: "", secretTouched: false });
        envChanged();
    }

    function removeEnvVar(index) {
        envModel.remove(index);
        envChanged();
    }

    // Sets one field of variable `index`; until «Geheim» is switched by hand, a new name switches it
    // on as `core` suggests, and off only while no value is typed (a typed secret never turns
    // visible by a rename).
    function editEnvVar(index, field, fieldValue) {
        envModel.setProperty(index, field, fieldValue);
        if (field === "secret") {
            envModel.setProperty(index, "secretTouched", true);
        } else if (field === "name" && !envModel.get(index).secretTouched) {
            var suggested = appBackend.envVarSecretByDefault(fieldValue);
            if (suggested || envModel.get(index).value === "") {
                envModel.setProperty(index, "secret", suggested);
            }
        }
        envChanged();
    }

    // Replaces all variables with `rows` (`{name, secret, value}`), each marked as switched by hand.
    function setEnvVars(rows) {
        envModel.clear();
        rows.forEach(function(v) {
            envModel.append({ name: v.name, secret: v.secret, value: v.value, secretTouched: true });
        });
        envChanged();
    }

    function fieldsJson() {
        var fields = {
            kind: kind,
            name: name.trim(),
            location: location.trim(),
            username: username.trim(),
            endpoint: endpoint.trim(),
            bucket: bucket.trim(),
            path: s3Path.trim(),
            region: region.trim(),
            env: kindIsGeneric ? envFields() : []
        };
        if (destinationId !== "") {
            fields.id = destinationId;
        }
        return JSON.stringify(fields);
    }

    // Backend credentials for `TestDestinationConnection`, `AddDestination` and
    // `UpdateDestination` (`BackendSecrets`); empty fields keep the saved values when editing.
    function backendSecretsJson() {
        if (kindIsS3) {
            return JSON.stringify({
                s3_access_key_id: accessKeyId.trim(),
                s3_secret_access_key: secretAccessKey.trim()
            });
        }
        if (kindIsRest) {
            return JSON.stringify({ rest_password: restPassword });
        }
        if (kindIsGeneric) {
            var env = {};
            envList().forEach(function(v) {
                if (v.secret && v.value.length > 0) {
                    env[v.name.trim()] = v.value;
                }
            });
            return JSON.stringify({ env: env });
        }
        return "{}";
    }

    // Fields that change where and how the repository is reached, to see whether they changed.
    function connectionKey() {
        return JSON.stringify([kind, location.trim(), username.trim(), endpoint.trim(), bucket.trim(),
            s3Path.trim(), region.trim(), envFields()]);
    }

    // Clears the entered passwords and keys after saving.
    function clearSecrets() {
        repoPassword = "";
        restPassword = "";
        accessKeyId = "";
        secretAccessKey = "";
        setEnvVars(envList().map(function(v) {
            return { name: v.name, secret: v.secret, value: v.secret ? "" : v.value };
        }));
    }

    // Fills the fields from a destination row (`DestinationsModel.rowMap`).
    function loadFromRow(data) {
        kind = data.kindName;
        name = data.name;
        location = data.kindName === "s3" ? "" : data.location;
        username = data.username;
        var detail = {};
        try {
            detail = JSON.parse(data.kindJson || "{}");
        } catch (e) {
            detail = {};
        }
        endpoint = detail.endpoint || "";
        bucket = detail.bucket || "";
        s3Path = detail.path || "";
        region = detail.region || "";
        setEnvVars((detail.env || []).map(function(v) {
            return { name: v.name, secret: !!v.secret, value: v.secret ? "" : (v.value || "") };
        }));
    }

    function showError(text) {
        testFeedbackOk = false;
        testFeedback = text;
    }

    function runConnectionTest() {
        testNote = "";
        testNoteConfirmed = false;
        if (!detailsValid) {
            showError(Texts.label("fill_required_fields"));
            return false;
        }
        var ok = appBackend.testDestination(fieldsJson(), setupMode, repoPassword, backendSecretsJson());
        testNote = ok ? appBackend.connection_test_note : "";
        connectionVerified = ok;
        testFeedbackOk = ok;
        testFeedback = appBackend.connection_test_message;
        return ok;
    }
}
