#!/bin/sh
set -eu

repo="${KAMISHIBAI_REPO:-anatoly-chichikov/kamishibai}"
requested="${KAMISHIBAI_VERSION:-latest}"
install_dir="${KAMISHIBAI_INSTALL_DIR:-$HOME/.local/bin}"

say() {
  printf '%s\n' "$*"
}

fail() {
  say "kamishibai install: $*" >&2
  exit 1
}

has() {
  command -v "$1" >/dev/null 2>&1
}

download() {
  url="$1"
  path="$2"
  if has curl; then
    curl -fsSL "$url" -o "$path"
    return
  fi
  if has wget; then
    wget -q "$url" -O "$path"
    return
  fi
  fail "curl or wget is required"
}

latest_tag() {
  tmp="$1"
  download "https://api.github.com/repos/$repo/releases/latest" "$tmp"
  sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp" | head -n 1
}

target() {
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$arch" in
    x86_64 | amd64)
      cpu="x86_64"
      ;;
    arm64 | aarch64)
      cpu="aarch64"
      ;;
    *)
      fail "unsupported architecture: $arch"
      ;;
  esac
  case "$os" in
    Darwin)
      archive_ext="tar.gz"
      binary="kamishibai"
      printf '%s|%s|%s\n' "$cpu-apple-darwin" "$archive_ext" "$binary"
      ;;
    Linux)
      [ "$cpu" = "x86_64" ] || fail "linux arm64 release asset is not available yet"
      archive_ext="tar.gz"
      binary="kamishibai"
      printf '%s|%s|%s\n' "$cpu-unknown-linux-gnu" "$archive_ext" "$binary"
      ;;
    MINGW* | MSYS* | CYGWIN*)
      [ "$cpu" = "x86_64" ] || fail "windows arm64 release asset is not available yet"
      archive_ext="zip"
      binary="kamishibai.exe"
      printf '%s|%s|%s\n' "$cpu-pc-windows-msvc" "$archive_ext" "$binary"
      ;;
    *)
      fail "unsupported operating system: $os"
      ;;
  esac
}

hash_file() {
  path="$1"
  if has sha256sum; then
    sha256sum "$path" | awk '{print $1}'
    return
  fi
  if has shasum; then
    shasum -a 256 "$path" | awk '{print $1}'
    return
  fi
  fail "sha256sum or shasum is required"
}

unpack() {
  archive="$1"
  archive_ext="$2"
  destination="$3"
  case "$archive_ext" in
    tar.gz)
      tar -xzf "$archive" -C "$destination"
      ;;
    zip)
      has unzip || fail "unzip is required for windows archives"
      unzip -q "$archive" -d "$destination"
      ;;
    *)
      fail "unsupported archive extension: $archive_ext"
      ;;
  esac
}

tmp="${TMPDIR:-/tmp}/kamishibai-install.$$"
mkdir -p "$tmp"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

case "$requested" in
  latest)
    tag="$(latest_tag "$tmp/latest.json")"
    ;;
  v*)
    tag="$requested"
    ;;
  *)
    tag="v$requested"
    ;;
esac

[ -n "$tag" ] || fail "could not resolve release tag"
version="${tag#v}"
target_info="$(target)"
release_target="$(printf '%s' "$target_info" | cut -d '|' -f 1)"
archive_ext="$(printf '%s' "$target_info" | cut -d '|' -f 2)"
binary="$(printf '%s' "$target_info" | cut -d '|' -f 3)"
asset="kamishibai-v${version}-${release_target}.${archive_ext}"
archive="$tmp/$asset"
checksums="$tmp/SHA256SUMS.txt"

base="https://github.com/$repo/releases/download/$tag"
say "downloading $asset"
download "$base/$asset" "$archive"
download "$base/SHA256SUMS.txt" "$checksums"

expected="$(awk -v name="$asset" '$2 == name {print $1}' "$checksums" | head -n 1)"
[ -n "$expected" ] || fail "checksum for $asset is missing"
actual="$(hash_file "$archive")"
[ "$actual" = "$expected" ] || fail "checksum mismatch for $asset"

unpack "$archive" "$archive_ext" "$tmp"
binary_path="$tmp/$binary"
[ -f "$binary_path" ] || fail "archive does not contain $binary"

mkdir -p "$install_dir"
cp "$binary_path" "$install_dir/$binary"
chmod 755 "$install_dir/$binary"

say "installed kamishibai $version to $install_dir/$binary"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) say "add $install_dir to PATH to run kamishibai from any shell" ;;
esac
