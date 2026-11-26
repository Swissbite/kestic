#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"

TOOLS_DIR="${KESTIC_TOOLS_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/kestic-dev/bin}"
CARGO_BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"

# Versions with a `# renovate:` line are updated by Renovate (.github/renovate.json).
# RESTIC_MIN_VERSION stays: the tests run against the oldest supported restic and the latest.
RESTIC_MIN_VERSION="0.18.0"
# renovate: datasource=github-releases depName=restic/restic
RESTIC_LATEST_VERSION="0.20.0"
# renovate: datasource=github-releases depName=restic/rest-server
REST_SERVER_VERSION="0.15.0"
# renovate: datasource=github-releases depName=rclone/rclone
RCLONE_VERSION="1.76.0"
# renovate: datasource=crate depName=cargo-deny
CARGO_DENY_VERSION="0.22.0"
# renovate: datasource=crate depName=cargo-deb
CARGO_DEB_VERSION="3.8.0"
# A new nextest version also needs the SHA-256 of its release archive below.
# renovate: datasource=crate depName=cargo-nextest
CARGO_NEXTEST_VERSION="0.9.152"
CARGO_NEXTEST_SHA256_X86_64="d039b43c3e0958441b6ad0a39c8884d018b3d2e8fb9928f209074b1100b5e17a"

# Official PGP key fingerprints (trust anchors)
RESTIC_PGP_FPR="CF8F18F2844575973F79D4E191A6868BD3F7A907"
RCLONE_PGP_FPR="FBF737ECE9F8AB18604BD2AC93935E02FF3B54FA"

APT_PACKAGES=(
    just
    reuse
    build-essential
    pkg-config
    cmake
    extra-cmake-modules
    lld
    gettext
    desktop-file-utils
    gnome-keyring
    locales
    procps
    qt6-base-dev
    qt6-declarative-dev
    qt6-tools-dev
    qt6-l10n-tools
    qt6-qpa-plugins
    qt6-svg-plugins
    libkirigami-dev
    kirigami-addons-dev
    qml6-module-org-kde-kirigami
    qml6-module-org-kde-kirigamiaddons-datetime
    qml6-module-org-kde-kirigamiaddons-tableview
    qml6-module-org-kde-desktop
    qml6-module-qtquick-controls
    qml6-module-qtquick-dialogs
    qml6-module-qtquick-layouts
    qml6-module-qtquick-templates
    qml6-module-qtquick-window
    qml6-module-qtqml-models
    breeze-icon-theme
    libkf6i18n-dev
    libkf6coreaddons-dev
    libkf6crash-dev
    libkf6iconthemes-dev
    libkf6configwidgets-dev
    libkf6kcmutils-dev
    libssl-dev
    libdbus-1-dev
    openssh-server
    dbus-x11
    curl
    bzip2
    gnupg
    unzip
)

ARCH_RAW="$(uname -m)"
case "$ARCH_RAW" in
    x86_64)
        GO_ARCH="amd64"
        RUST_TARGET="x86_64-unknown-linux-gnu"
        ;;
    aarch64)
        GO_ARCH="arm64"
        RUST_TARGET="aarch64-unknown-linux-gnu"
        ;;
    *)
        echo "Unsupported architecture: $ARCH_RAW" >&2
        exit 1
        ;;
esac

COMPONENTS="${KESTIC_BOOTSTRAP_COMPONENTS:-all}"
has_component() {
    local name="$1"
    [[ "$COMPONENTS" == "all" ]] || [[ ",$COMPONENTS," == *",$name,"* ]]
}

TMP_DIR="$(mktemp -d)"
cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT

import_trusted_gpg_key() {
    local fpr="$1"
    local gnupghome="$2"
    local key_file="$TMP_DIR/key-${fpr}.asc"

    mkdir -p "$gnupghome"
    chmod 700 "$gnupghome"

    if [[ "${KESTIC_TEST_MOCK_URLS:-0}" == "1" && -n "${KESTIC_GPG_PUBKEY_FILE:-}" ]]; then
        cp "$KESTIC_GPG_PUBKEY_FILE" "$key_file"
    else
        if ! curl -fsSL "https://keyserver.ubuntu.com/pks/lookup?op=get&search=0x${fpr}" -o "$key_file"; then
            curl -fsSL "https://keys.openpgp.org/vks/v1/by-fingerprint/${fpr}" -o "$key_file"
        fi
    fi

    GNUPGHOME="$gnupghome" gpg --batch --quiet --import "$key_file"

    local imported_fpr
    imported_fpr="$(GNUPGHOME="$gnupghome" gpg --batch --with-colons --fingerprint "$fpr" \
        | awk -F: '$1 == "fpr" { print $10; exit }')"
    if [[ "$imported_fpr" != "$fpr" ]]; then
        echo "PGP key fingerprint mismatch: expected $fpr, got ${imported_fpr:-<none>}" >&2
        exit 1
    fi
}

verify_detached_pgp() {
    local fpr="$1"
    local sig_file="$2"
    local data_file="$3"
    local gnupghome="$TMP_DIR/gnupg-${fpr}"

    import_trusted_gpg_key "$fpr" "$gnupghome"

    local status_out
    if ! status_out="$(GNUPGHOME="$gnupghome" gpg --batch --status-fd 1 --verify "$sig_file" "$data_file" 2>&1)"; then
        echo "PGP signature verification failed for $data_file" >&2
        echo "$status_out" >&2
        exit 1
    fi

    if ! grep -q "VALIDSIG .* ${fpr}$" <<<"$status_out" && ! grep -q "VALIDSIG ${fpr} " <<<"$status_out"; then
        echo "PGP signature did not match expected primary fingerprint $fpr" >&2
        echo "$status_out" >&2
        exit 1
    fi
}

verify_clearsigned_pgp() {
    local fpr="$1"
    local signed_file="$2"
    local out_file="$3"
    local gnupghome="$TMP_DIR/gnupg-${fpr}"

    import_trusted_gpg_key "$fpr" "$gnupghome"

    local status_out
    if ! status_out="$(GNUPGHOME="$gnupghome" gpg --batch --status-fd 1 --output "$out_file" --decrypt "$signed_file" 2>&1)"; then
        echo "PGP clearsigned verification failed for $signed_file" >&2
        echo "$status_out" >&2
        exit 1
    fi

    if ! grep -q "VALIDSIG .* ${fpr}$" <<<"$status_out" && ! grep -q "VALIDSIG ${fpr} " <<<"$status_out"; then
        echo "PGP clearsigned signature did not match expected primary fingerprint $fpr" >&2
        echo "$status_out" >&2
        exit 1
    fi
}

verify_sha256_entry() {
    local sums_file="$1"
    local filename="$2"
    local workdir="$3"

    local expected_line
    expected_line="$(grep -E "^[0-9a-f]{64}  ${filename}$" "$sums_file" || true)"
    if [[ -z "$expected_line" ]]; then
        echo "No SHA256 entry found for $filename in $sums_file" >&2
        exit 1
    fi

    (
        cd "$workdir"
        printf '%s\n' "$expected_line" | sha256sum -c -
    )
}

ensure_apt_packages() {
    if [[ "${KESTIC_BOOTSTRAP_SKIP_APT:-0}" == "1" ]]; then
        return 0
    fi

    local missing=()
    local pkg
    for pkg in "${APT_PACKAGES[@]}"; do
        if ! dpkg -s "$pkg" >/dev/null 2>&1; then
            missing+=("$pkg")
        fi
    done

    if [[ ${#missing[@]} -eq 0 ]]; then
        echo "[ok] All required apt packages already installed"
        return 0
    fi

    echo "[apt] Missing packages: ${missing[*]}"
    if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
        apt-get update
        apt-get install -y "${missing[@]}"
    else
        sudo apt-get update
        sudo apt-get install -y "${missing[@]}"
    fi
}

ensure_rust_toolchain() {
    local channel
    channel="$(awk -F'"' '/^channel[[:space:]]*=/{print $2; exit}' "$REPO_ROOT/rust-toolchain.toml")"
    if [[ -z "$channel" ]]; then
        echo "Could not parse toolchain channel from rust-toolchain.toml" >&2
        exit 1
    fi

    if [[ ! -x "$CARGO_BIN_DIR/rustup" ]]; then
        local base_url="https://static.rust-lang.org/rustup/dist/${RUST_TARGET}"
        if [[ "${KESTIC_TEST_MOCK_URLS:-0}" == "1" && -n "${KESTIC_RUSTUP_BASE_URL:-}" ]]; then
            base_url="$KESTIC_RUSTUP_BASE_URL"
        fi

        echo "[rustup] Downloading rustup-init from $base_url"
        curl -fsSL "${base_url}/rustup-init" -o "$TMP_DIR/rustup-init"
        curl -fsSL "${base_url}/rustup-init.sha256" -o "$TMP_DIR/rustup-init.sha256"
        (
            cd "$TMP_DIR"
            sed 's|\*./rustup-init|rustup-init|' rustup-init.sha256 | sha256sum -c -
        )
        chmod +x "$TMP_DIR/rustup-init"
        "$TMP_DIR/rustup-init" -y --no-modify-path --default-toolchain none
    else
        echo "[ok] rustup already installed"
    fi

    export PATH="$CARGO_BIN_DIR:$PATH"

    if ! rustup toolchain list | grep -q "^${channel}-"; then
        echo "[rustup] Installing toolchain $channel (rustfmt, clippy)"
        rustup toolchain install "$channel" --component rustfmt --component clippy --profile minimal
    else
        local missing_comp=0
        for comp in rustfmt clippy; do
            if ! rustup component list --toolchain "$channel" --installed | grep -q "^${comp}"; then
                missing_comp=1
            fi
        done
        if [[ "$missing_comp" -eq 1 ]]; then
            rustup component add --toolchain "$channel" rustfmt clippy
        else
            echo "[ok] Rust toolchain $channel (rustfmt, clippy) already installed"
        fi
    fi

    if ! "$CARGO_BIN_DIR/cargo-deny" --version 2>/dev/null | grep -q "${CARGO_DENY_VERSION}"; then
        echo "[cargo] Installing cargo-deny $CARGO_DENY_VERSION"
        cargo "+$channel" install --locked cargo-deny --version "$CARGO_DENY_VERSION"
    else
        echo "[ok] cargo-deny $CARGO_DENY_VERSION already installed"
    fi

    if ! "$CARGO_BIN_DIR/cargo-deb" --version 2>/dev/null | grep -q "${CARGO_DEB_VERSION}"; then
        echo "[cargo] Installing cargo-deb $CARGO_DEB_VERSION"
        cargo "+$channel" install --locked cargo-deb --version "$CARGO_DEB_VERSION"
    else
        echo "[ok] cargo-deb $CARGO_DEB_VERSION already installed"
    fi

    if ! "$CARGO_BIN_DIR/cargo-nextest" --version 2>/dev/null | grep -q "${CARGO_NEXTEST_VERSION}"; then
        if [[ "$RUST_TARGET" == "x86_64-unknown-linux-gnu" && "${KESTIC_TEST_MOCK_URLS:-0}" != "1" ]]; then
            echo "[cargo] Installing prebuilt cargo-nextest $CARGO_NEXTEST_VERSION"
            local nextest_archive="cargo-nextest-${CARGO_NEXTEST_VERSION}-${RUST_TARGET}.tar.gz"
            local nextest_url="https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-${CARGO_NEXTEST_VERSION}/${nextest_archive}"
            curl -fsSL "$nextest_url" -o "$TMP_DIR/$nextest_archive"
            (
                cd "$TMP_DIR"
                printf '%s  %s\n' "$CARGO_NEXTEST_SHA256_X86_64" "$nextest_archive" | sha256sum -c -
            )
            mkdir -p "$CARGO_BIN_DIR"
            tar -xzf "$TMP_DIR/$nextest_archive" -C "$CARGO_BIN_DIR" cargo-nextest
            chmod 755 "$CARGO_BIN_DIR/cargo-nextest"
        else
            echo "[cargo] Installing cargo-nextest $CARGO_NEXTEST_VERSION"
            cargo "+$channel" install --locked cargo-nextest --version "$CARGO_NEXTEST_VERSION"
        fi
    else
        echo "[ok] cargo-nextest $CARGO_NEXTEST_VERSION already installed"
    fi
}

install_restic_version() {
    local version="$1"
    local dest="$TOOLS_DIR/restic-${version}"

    if [[ -x "$dest" ]] && "$dest" version 2>/dev/null | grep -q "^restic ${version} "; then
        echo "[ok] restic ${version} already installed at $dest"
        return 0
    fi

    local base_url="https://github.com/restic/restic/releases/download/v${version}"
    local fpr="$RESTIC_PGP_FPR"
    if [[ "${KESTIC_TEST_MOCK_URLS:-0}" == "1" && -n "${KESTIC_RESTIC_BASE_URL:-}" ]]; then
        base_url="$KESTIC_RESTIC_BASE_URL"
        fpr="${KESTIC_GPG_TRUSTED_FPR:-$RESTIC_PGP_FPR}"
    fi

    local archive="restic_${version}_linux_${GO_ARCH}.bz2"
    local work="$TMP_DIR/restic-${version}"
    mkdir -p "$work"

    echo "[restic] Downloading restic ${version} (${GO_ARCH})"
    curl -fsSL "${base_url}/${archive}" -o "$work/${archive}"
    curl -fsSL "${base_url}/SHA256SUMS" -o "$work/SHA256SUMS"
    curl -fsSL "${base_url}/SHA256SUMS.asc" -o "$work/SHA256SUMS.asc"

    verify_detached_pgp "$fpr" "$work/SHA256SUMS.asc" "$work/SHA256SUMS"
    verify_sha256_entry "$work/SHA256SUMS" "$archive" "$work"

    bzip2 -dc "$work/${archive}" > "$work/restic.bin"
    chmod 755 "$work/restic.bin"
    mkdir -p "$TOOLS_DIR"
    mv "$work/restic.bin" "$dest"
    echo "[ok] Installed $dest"
}

install_rest_server() {
    local version="$REST_SERVER_VERSION"
    local dest="$TOOLS_DIR/rest-server"
    local marker="$TOOLS_DIR/.rest-server.version"

    if [[ -x "$dest" && -f "$marker" ]] && [[ "$(cat "$marker")" == "$version" ]]; then
        echo "[ok] rest-server ${version} already installed at $dest"
        return 0
    fi

    local base_url="https://github.com/restic/rest-server/releases/download/v${version}"
    local fpr="$RESTIC_PGP_FPR"
    if [[ "${KESTIC_TEST_MOCK_URLS:-0}" == "1" && -n "${KESTIC_REST_SERVER_BASE_URL:-}" ]]; then
        base_url="$KESTIC_REST_SERVER_BASE_URL"
        fpr="${KESTIC_GPG_TRUSTED_FPR:-$RESTIC_PGP_FPR}"
    fi

    local archive="rest-server_${version}_linux_${GO_ARCH}.tar.gz"
    local work="$TMP_DIR/rest-server"
    mkdir -p "$work"

    echo "[rest-server] Downloading rest-server ${version} (${GO_ARCH})"
    curl -fsSL "${base_url}/${archive}" -o "$work/${archive}"
    curl -fsSL "${base_url}/SHA256SUMS" -o "$work/SHA256SUMS"
    curl -fsSL "${base_url}/SHA256SUMS.asc" -o "$work/SHA256SUMS.asc"

    verify_detached_pgp "$fpr" "$work/SHA256SUMS.asc" "$work/SHA256SUMS"
    verify_sha256_entry "$work/SHA256SUMS" "$archive" "$work"

    tar -xzf "$work/${archive}" -C "$work"
    local extracted_bin
    extracted_bin="$(find "$work" -type f -name rest-server | head -n 1)"
    if [[ -z "$extracted_bin" ]]; then
        echo "Could not find rest-server binary inside $archive" >&2
        exit 1
    fi
    chmod 755 "$extracted_bin"
    mkdir -p "$TOOLS_DIR"
    mv "$extracted_bin" "$dest"
    printf '%s\n' "$version" > "$marker"
    echo "[ok] Installed $dest ($version)"
}

install_rclone() {
    local version="$RCLONE_VERSION"
    local dest="$TOOLS_DIR/rclone"
    local marker="$TOOLS_DIR/.rclone.version"

    if [[ -x "$dest" && -f "$marker" ]] && [[ "$(cat "$marker")" == "$version" ]]; then
        echo "[ok] rclone v${version} already installed at $dest"
        return 0
    fi
    if [[ -x "$dest" ]] && "$dest" version 2>/dev/null | grep -q "v${version}$"; then
        printf '%s\n' "$version" > "$marker"
        echo "[ok] rclone v${version} already installed at $dest"
        return 0
    fi

    local base_url="https://downloads.rclone.org/v${version}"
    local fpr="$RCLONE_PGP_FPR"
    if [[ "${KESTIC_TEST_MOCK_URLS:-0}" == "1" && -n "${KESTIC_RCLONE_BASE_URL:-}" ]]; then
        base_url="$KESTIC_RCLONE_BASE_URL"
        fpr="${KESTIC_GPG_TRUSTED_FPR:-$RCLONE_PGP_FPR}"
    fi

    local archive="rclone-v${version}-linux-${GO_ARCH}.zip"
    local work="$TMP_DIR/rclone"
    mkdir -p "$work"

    echo "[rclone] Downloading rclone v${version} (${GO_ARCH})"
    curl -fsSL "${base_url}/${archive}" -o "$work/${archive}"
    curl -fsSL "${base_url}/SHA256SUMS" -o "$work/SHA256SUMS.asc"

    verify_clearsigned_pgp "$fpr" "$work/SHA256SUMS.asc" "$work/SHA256SUMS"
    verify_sha256_entry "$work/SHA256SUMS" "$archive" "$work"

    unzip -q "$work/${archive}" -d "$work"
    local extracted_bin
    extracted_bin="$(find "$work" -type f -name rclone | head -n 1)"
    if [[ -z "$extracted_bin" ]]; then
        echo "Could not find rclone binary inside $archive" >&2
        exit 1
    fi
    chmod 755 "$extracted_bin"
    mkdir -p "$TOOLS_DIR"
    mv "$extracted_bin" "$dest"
    printf '%s\n' "$version" > "$marker"
    echo "[ok] Installed $dest (v${version})"
}

if has_component "apt"; then
    ensure_apt_packages
fi

if has_component "rust"; then
    ensure_rust_toolchain
fi

if has_component "restic-0.18.0"; then
    install_restic_version "$RESTIC_MIN_VERSION"
fi

if has_component "restic-latest"; then
    install_restic_version "$RESTIC_LATEST_VERSION"
    if [[ "$(readlink "$TOOLS_DIR/restic-latest" 2>/dev/null || true)" != "restic-${RESTIC_LATEST_VERSION}" ]]; then
        ln -sfn "restic-${RESTIC_LATEST_VERSION}" "$TOOLS_DIR/restic-latest"
    fi
    if [[ "$(readlink "$TOOLS_DIR/restic" 2>/dev/null || true)" != "restic-${RESTIC_LATEST_VERSION}" ]]; then
        ln -sfn "restic-${RESTIC_LATEST_VERSION}" "$TOOLS_DIR/restic"
    fi
fi

if has_component "rest-server"; then
    install_rest_server
fi

if has_component "rclone"; then
    install_rclone
fi

echo "Bootstrap complete. Test tools installed in: $TOOLS_DIR"
