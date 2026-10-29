#!/bin/sh
# Download and verify an OpenCodifier release binary. No Rust toolchain needed.
set -eu

VERSION=${OPENCODIFIER_VERSION:-v0.6.2}
INSTALL_DIR=${OPENCODIFIER_INSTALL_DIR:-${HOME:?HOME must be set}/.local/bin}
RELEASE_BASE_URL=${OPENCODIFIER_RELEASE_BASE_URL:-https://github.com/aliasfoxkde/OpenCodifier/releases/download}

usage() {
  cat <<'USAGE'
Usage: scripts/install-prebuilt.sh [--version VERSION] [--install-dir DIR]

Downloads the prebuilt deterministic OpenCodifier CLI and its release
attestation, verifies both, checks the reported CLI version, and atomically
installs it as opencodifier. No Rust/Cargo installation or service startup.

Options can also be set with OPENCODIFIER_VERSION and OPENCODIFIER_INSTALL_DIR.
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      [ "$#" -ge 2 ] || { usage >&2; exit 2; }
      VERSION=$2
      shift 2
      ;;
    --install-dir)
      [ "$#" -ge 2 ] || { usage >&2; exit 2; }
      INSTALL_DIR=$2
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "$VERSION" in
  v[0-9]*.[0-9]*.[0-9]*) VERSION_NUMBER=${VERSION#v} ;;
  [0-9]*.[0-9]*.[0-9]*) VERSION_NUMBER=$VERSION; VERSION=v$VERSION ;;
  *) echo "Version must be a stable numeric version, for example v0.6.2" >&2; exit 2 ;;
esac
case "$VERSION_NUMBER" in
  *[!0-9.]*|.*|*..*|*.) echo "Invalid version: $VERSION" >&2; exit 2 ;;
esac
DOT_COUNT=$(printf '%s' "$VERSION_NUMBER" | tr -cd '.' | wc -c | tr -d ' ')
[ "$DOT_COUNT" -eq 2 ] || { echo "Version must have major.minor.patch form: $VERSION" >&2; exit 2; }

command -v curl >/dev/null 2>&1 || { echo "curl is required to download the release" >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "Python 3 is required for standard-library attestation verification; Rust is not required" >&2; exit 1; }
command -v uname >/dev/null 2>&1 || { echo "uname is required for platform detection" >&2; exit 1; }

OS=$(uname -s)
ARCH=$(uname -m)
case "$OS:$ARCH" in
  Linux:x86_64|Linux:amd64) TARGET=x86_64-unknown-linux-musl ;;
  Linux:aarch64|Linux:arm64) TARGET=aarch64-unknown-linux-gnu ;;
  Darwin:x86_64|Darwin:amd64) TARGET=x86_64-apple-darwin ;;
  Darwin:arm64|Darwin:aarch64) TARGET=aarch64-apple-darwin ;;
  *) echo "No first-party prebuilt CLI installer target for $OS/$ARCH" >&2; exit 1 ;;
esac

ASSET="opencodifier-${VERSION_NUMBER}-${TARGET}"
URL="${RELEASE_BASE_URL%/}/${VERSION}/${ASSET}"
TMP_ROOT=${TMPDIR:-/tmp}
TMP_DIR=$(mktemp -d "$TMP_ROOT/opencodifier-install.XXXXXX")
cleanup() { rm -rf "$TMP_DIR"; }
trap cleanup EXIT HUP INT TERM

printf 'Downloading OpenCodifier %s for %s\n' "$VERSION_NUMBER" "$TARGET"
curl --fail --location --silent --show-error --retry 3 --connect-timeout 15 --max-time 180 "$URL" -o "$TMP_DIR/opencodifier"
curl --fail --location --silent --show-error --retry 3 --connect-timeout 15 --max-time 60 "${URL}.attestation.json" -o "$TMP_DIR/attestation.json"
python3 "$(dirname "$0")/verify_prebuilt_attestation.py" "$TMP_DIR/attestation.json" "$TMP_DIR/opencodifier" "$TARGET"
chmod 755 "$TMP_DIR/opencodifier"
REPORTED_VERSION=$("$TMP_DIR/opencodifier" --version)
EXPECTED_VERSION="opencodifier $VERSION_NUMBER"
if [ "$REPORTED_VERSION" != "$EXPECTED_VERSION" ]; then
  echo "Binary reports '$REPORTED_VERSION'; expected '$EXPECTED_VERSION'" >&2
  exit 1
fi

mkdir -p "$INSTALL_DIR"
TEMP_INSTALL="$INSTALL_DIR/.opencodifier.$$"
trap 'rm -f "$TEMP_INSTALL"; cleanup' EXIT HUP INT TERM
cp "$TMP_DIR/opencodifier" "$TEMP_INSTALL"
chmod 755 "$TEMP_INSTALL"
mv -f "$TEMP_INSTALL" "$INSTALL_DIR/opencodifier"
printf 'Installed %s to %s/opencodifier\n' "$REPORTED_VERSION" "$INSTALL_DIR"
printf 'No service was started. Add %s to PATH if needed.\n' "$INSTALL_DIR"
