#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
# SPDX-License-Identifier: BSD-2-Clause

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"

TOOLS_DIR="${KESTIC_TOOLS_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/kestic-dev/bin}"
CARGO_BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"

RESTIC_MIN_VERSION="0.18.0"
RESTIC_LATEST_VERSION="0.19.1"
REST_SERVER_VERSION="0.14.0"
RCLONE_VERSION="1.76.0"
CARGO_DENY_VERSION="0.20.2"
CARGO_DEB_VERSION="3.8.0"

# Official PGP key fingerprints (trust anchors)
RESTIC_PGP_FPR="CF8F18F2844575973F79D4E191A6868BD3F7A907"
RCLONE_PGP_FPR="FBF737ECE9F8AB18604BD2AC93935E02FF3B54FA"

APT_PACKAGES=(
    just
    reuse
    cmake
    extra-cmake-modules
    qt6-base-dev
    qt6-declarative-dev
    qt6-tools-dev
    qt6-l10n-tools
    libkirigami-dev
    kirigami-addons-dev
    libkf6i18n-dev
    libkf6coreaddons-dev
    libkf6crash-dev
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
    sudo apt-get update
    sudo apt-get install -y "${missing[@]}"
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
        cargo install --locked cargo-deny --version "$CARGO_DENY_VERSION"
    else
        echo "[ok] cargo-deny $CARGO_DENY_VERSION already installed"
    fi

    if ! "$CARGO_BIN_DIR/cargo-deb" --version 2>/dev/null | grep -q "${CARGO_DEB_VERSION}"; then
        echo "[cargo] Installing cargo-deb $CARGO_DEB_VERSION"
        cargo install --locked cargo-deb --version "$CARGO_DEB_VERSION"
    else
        echo "[ok] cargo-deb $CARGO_DEB_VERSION already installed"
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

    if [[ -x "$dest" ]] && "$dest" version 2>/dev/null | head -n 1 | grep -q "v${version}$"; then
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
