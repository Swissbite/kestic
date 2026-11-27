// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

import QtQuick
import net.daester.kestic 1.0

// Fields, checks and connection test of a destination that is being added or edited.
QtObject {
    id: draft

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
    // SFTP server: host, port and path; the user is `username`.
    property string host: ""
    property string port: "22"
    property string sftpPath: ""
    // `HostKeyDto` of the last connection test that met an unknown server, else null.
    property var hostKey: null
    // Generic form (B2, Azure, Google Cloud, Swift, rclone): the repository URL is `location`;
    // the variables as rows `{name, secret, value, secretTouched}`. A row is changed in place, so
    // the field being typed in keeps its focus; `envRevision` counts the changes for bindings.
    // `secretTouched`: «Geheim» was switched by hand and no longer follows the name.
    property ListModel envModel: ListModel {}
    property int envRevision: 0
    // Names of the secret variables whose value the password store holds for `destinationId`:
    // only such a row, left empty, keeps a saved value.
    property var storedSecretNames: []
    property string repoPassword: ""
    property string repoPasswordConfirm: ""
    property bool connectionVerified: false
    property string testFeedback: ""
    property bool testFeedbackOk: false
    // Warning of the last successful connection test (a folder on a CIFS/SMB share) and whether
    // the user confirmed it.
    property string testNote: ""
    property bool testNoteConfirmed: false

    // The last connection test met a server whose host key no `known_hosts` file has.
    signal hostKeyOffered()

    readonly property bool kindIsRest: kind === "rest"
    readonly property bool kindIsS3: kind === "s3"
    readonly property bool kindIsSftp: kind === "sftp"
    readonly property bool kindIsGeneric: kind === "generic"
    // The rclone card asks for the remote only; the URL is `rclone:` and the remote.
    readonly property bool asksRcloneRemote: kindIsGeneric && typeCard === "rclone"
    // Why `core` refuses the URL and the variables of the generic form (`{url, env}`, each empty
    // when accepted), also when an `azure:` or `gs:` destination lacks its credential variable.
    readonly property var genericErrors: kindIsGeneric
        ? JSON.parse(appBackend.genericFieldsErrors(location, JSON.stringify(envChecks())))
        : ({ url: "", env: "" })
    readonly property bool nameValid: name.trim().length > 0
    readonly property bool locationValid: kindIsS3
        ? endpointValid && bucketValid && s3PathValid && regionValid
        : kindIsSftp ? hostValid && portValid && sftpUserValid && sftpPathValid
        : kindIsGeneric
            ? genericErrors.url === "" && genericErrors.env === ""
            : kindIsRest
                ? /^https?:\/\//i.test(location.trim())
                : location.trim().length > 0
    // Which S3 and SFTP fields `core` accepts.
    readonly property var fieldChecks: JSON.parse(appBackend.destinationFieldChecks(JSON.stringify({
        endpoint: endpoint, bucket: bucket, s3Path: s3Path, region: region,
        host: host, port: port, username: username, sftpPath: sftpPath
    })))
    readonly property bool endpointValid: fieldChecks.endpoint
    readonly property bool bucketValid: fieldChecks.bucket
    readonly property bool s3PathValid: fieldChecks.s3Path
    readonly property bool regionValid: fieldChecks.region
    readonly property bool hostValid: fieldChecks.host
    readonly property bool portValid: fieldChecks.port
    readonly property bool sftpUserValid: fieldChecks.username
    readonly property bool sftpPathValid: fieldChecks.sftpPath
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
        || envModel.count > 0 || host.trim().length > 0 || sftpPath.trim().length > 0

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

    // The variables for `genericErrors`, with the typed values of the secret ones; an empty
    // secret value keeps the saved one when editing, if one is saved under that name.
    function envChecks() {
        return envList().map(function(v) {
            var name = v.name.trim();
            return {
                name: name, secret: v.secret, value: v.value,
                saved: v.secret && v.value.length === 0 && destinationId !== ""
                    && storedSecretNames.indexOf(name) >= 0
            };
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
            path: kindIsSftp ? sftpPath.trim() : s3Path.trim(),
            region: region.trim(),
            host: host.trim(),
            port: portValid ? Number(port) : 22,
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
            s3Path.trim(), region.trim(), host.trim(), port.trim(), sftpPath.trim(), envFields()]);
    }

    // Clears the entered passwords and keys after saving.
    function clearSecrets() {
        repoPassword = "";
        restPassword = "";
        accessKeyId = "";
        secretAccessKey = "";
        storedSecretNames = envList().filter(function(v) {
            var name = v.name.trim();
            return v.secret && (v.value.length > 0 || storedSecretNames.indexOf(name) >= 0);
        }).map(function(v) { return v.name.trim(); });
        setEnvVars(envList().map(function(v) {
            return { name: v.name, secret: v.secret, value: v.secret ? "" : v.value };
        }));
    }

    // Fills the fields from a destination row (`DestinationsModel.rowMap`).
    function loadFromRow(data) {
        kind = data.kindName;
        name = data.name;
        location = data.kindName === "s3" || data.kindName === "sftp" ? "" : data.location;
        username = data.username;
        var detail = {};
        try {
            detail = JSON.parse(data.kindJson || "{}");
        } catch (e) {
            detail = {};
        }
        endpoint = detail.endpoint || "";
        bucket = detail.bucket || "";
        s3Path = data.kindName === "s3" ? (detail.path || "") : "";
        sftpPath = data.kindName === "sftp" ? (detail.path || "") : "";
        host = detail.host || "";
        port = String(detail.port || 22);
        region = detail.region || "";
        storedSecretNames = (detail.env || []).filter(function(v) { return !!v.secret; })
            .map(function(v) { return v.name; });
        setEnvVars((detail.env || []).map(function(v) {
            return { name: v.name, secret: !!v.secret, value: v.secret ? "" : (v.value || "") };
        }));
    }

    // The entered connection without secrets and without the destination's ID, for adding it as
    // a new destination.
    function prefillData() {
        return {
            kind: kind,
            typeCard: kind === "local" ? "folder"
                : kind === "s3" ? "cloud"
                : kind === "generic" ? (location.trim().indexOf("rclone:") === 0 ? "rclone" : "cloud")
                : kind,
            setupMode: setupMode,
            name: name.trim(),
            location: location.trim(),
            username: username.trim(),
            endpoint: endpoint.trim(),
            bucket: bucket.trim(),
            s3Path: s3Path.trim(),
            region: region.trim(),
            host: host.trim(),
            port: port.trim(),
            sftpPath: sftpPath.trim(),
            env: envFields()
        };
    }

    // Fills the fields from `prefillData()` of another draft.
    function loadPrefill(data) {
        kind = data.kind;
        typeCard = data.typeCard;
        setupMode = data.setupMode;
        name = data.name;
        location = data.location;
        username = data.username;
        endpoint = data.endpoint;
        bucket = data.bucket;
        s3Path = data.s3Path;
        region = data.region;
        host = data.host;
        port = data.port;
        sftpPath = data.sftpPath;
        storedSecretNames = [];
        setEnvVars(data.env);
        connectionVerified = false;
    }

    // The trust request the draft waits for before it tests again.
    property int pendingTrust: 0

    property Connections trustAnswer: Connections {
        target: draft.appBackend
        function onDaemonCallDone(request, ok, result) {
            if (request !== draft.pendingTrust) return;
            draft.pendingTrust = 0;
            draft.runConnectionTest();
        }
    }

    // Trusts the host key of the last connection test and tests again once the daemon answered.
    function trustHostKey() {
        var id = hostKey ? hostKey.host_key_id : "";
        hostKey = null;
        // Trusted or not (the shown key expired, the daemon restarted, a file got another key
        // meanwhile): a new test shows where the server stands now.
        pendingTrust = appBackend.trustHostKey(id);
        if (pendingTrust === 0) {
            runConnectionTest();
        }
    }

    function showError(text) {
        testFeedbackOk = false;
        testFeedback = text;
    }

    function runConnectionTest() {
        testNote = "";
        testNoteConfirmed = false;
        hostKey = null;
        if (!detailsValid) {
            showError(Texts.label("fill_required_fields"));
            return;
        }
        connectionVerified = false;
        testFeedbackOk = false;
        testFeedback = "";
        testAnswer.track(appBackend.testDestination(fieldsJson(), setupMode, repoPassword, backendSecretsJson()));
    }

    // The connection test is on its way to the daemon.
    readonly property bool testing: testAnswer.waiting

    property DaemonAnswer testAnswer: DaemonAnswer {
        backend: draft.appBackend
        onAnswered: function(ok, result) {
            draft.showTestResult(ok);
        }
    }

    // Shows the result of the connection test the backend published.
    function showTestResult(ok) {
        testNote = ok ? appBackend.connection_test_note : "";
        connectionVerified = ok;
        testFeedbackOk = ok;
        testFeedback = appBackend.connection_test_message;
        if (appBackend.connection_test_host_key !== "") {
            hostKey = JSON.parse(appBackend.connection_test_host_key);
            hostKeyOffered();
        }
    }
}
