#!/bin/sh
# Installe la dernière version d'EasyTab (Linux, macOS) :
#   curl -fsSL https://github.com/AdamSellin/EasyTab/releases/latest/download/install.sh | sh
# Dépôt privé : depuis un clone du dépôt, `sh scripts/install.sh`. Le script se
# sert alors des identifiants GitHub de git (ou de $GITHUB_TOKEN, ou de gh).
# Variables : EASYTAB_VERSION (ex. v0.2.0, défaut : la dernière), EASYTAB_SHELL (zsh ou bash),
# EASYTAB_LANG (fr ou en, défaut : selon la locale).
set -eu

# Messages en anglais, ou en français si la locale l'est (EASYTAB_LANG passe
# avant, comme pour easytab lui-même).
lang="${EASYTAB_LANG:-${LC_ALL:-${LC_MESSAGES:-${LANG:-}}}}"
case "$lang" in
  fr*|FR*) fr=1 ;;
  *) fr= ;;
esac
# say "english" "français" : affiche le message dans la langue choisie.
say() {
  if [ -n "$fr" ]; then printf '%s\n' "$2"; else printf '%s\n' "$1"; fi
}

repo="AdamSellin/EasyTab"
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target="x86_64-unknown-linux-gnu" ;;
  Darwin-arm64) target="aarch64-apple-darwin" ;;
  Darwin-x86_64) target="x86_64-apple-darwin" ;;
  *)
    say "easytab: unsupported system ($(uname -s) $(uname -m))" \
      "easytab : système non pris en charge ($(uname -s) $(uname -m))" >&2
    exit 1 ;;
esac

asset="easytab-$target.tar.gz"
if [ -n "${EASYTAB_VERSION:-}" ]; then
  url="https://github.com/$repo/releases/download/$EASYTAB_VERSION/$asset"
  release="tags/$EASYTAB_VERSION"
else
  url="https://github.com/$repo/releases/latest/download/$asset"
  release="latest"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
archive="$tmp/easytab.tar.gz"

# Dépôt privé : le lien direct répond 404. On télécharge alors par l'API
# GitHub avec un jeton : $GITHUB_TOKEN, ou celui que git utilise déjà.
# Usage: private_download <asset> <file>
private_download() {
  token="${GITHUB_TOKEN:-}"
  if [ -z "$token" ] && command -v git >/dev/null 2>&1; then
    token="$(printf 'protocol=https\nhost=github.com\n\n' \
      | GIT_TERMINAL_PROMPT=0 git credential fill 2>/dev/null | sed -n 's/^password=//p')"
  fi
  [ -n "$token" ] || return 1
  asset_url="$(curl -fsSL -H "Authorization: Bearer $token" \
      "https://api.github.com/repos/$repo/releases/$release" \
    | tr -d '\n' | sed 's/}, *{/}\n{/g' | grep "\"name\": *\"$1\"" \
    | sed -n 's/.*"url": *"\(https:[^"]*\/releases\/assets\/[0-9]*\)".*/\1/p' | head -n 1)"
  [ -n "$asset_url" ] || return 1
  curl -fsSL -H "Authorization: Bearer $token" -H "Accept: application/octet-stream" \
    "$asset_url" -o "$2"
}

# Usage: fetch <asset> <file>. Direct link first, then the GitHub API
# (private repository), then gh. Returns 1 if every way failed.
fetch() {
  curl -fsSL "${url%/*}/$1" -o "$2" 2>/dev/null && return 0
  private_download "$1" "$2" && return 0
  command -v gh >/dev/null 2>&1 \
    && gh release download ${EASYTAB_VERSION:-} -R "$repo" -p "$1" -O "$2" 2>/dev/null
}

say "Downloading $url" "Téléchargement de $url"
if ! fetch "$asset" "$archive"; then
  say "easytab: download failed. Private repository: connect git to GitHub, or set GITHUB_TOKEN." \
    "easytab : téléchargement impossible. Dépôt privé : connecte git à GitHub, ou définis GITHUB_TOKEN." >&2
  exit 1
fi

# Check the archive against the release's SHA256SUMS before extracting it.
# Releases before SHA256SUMS existed (v0.1.11 and older): warn and go on.
sums="$tmp/SHA256SUMS"
if fetch SHA256SUMS "$sums"; then
  expected="$(awk -v f="$asset" '$2 == f || $2 == "*" f { print tolower($1); exit }' "$sums")"
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum < "$archive" | cut -d' ' -f1)"
  else
    actual="$(shasum -a 256 < "$archive" | cut -d' ' -f1)"
  fi
  if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
    say "easytab: $asset does not match its SHA-256 in SHA256SUMS (download corrupted or tampered with). Nothing was installed." \
      "easytab : $asset ne correspond pas à son SHA-256 dans SHA256SUMS (téléchargement corrompu ou modifié). Rien n'a été installé." >&2
    exit 1
  fi
  say "SHA-256 checked" "SHA-256 vérifié"
else
  say "easytab: no SHA256SUMS in this release, archive not checked" \
    "easytab : pas de SHA256SUMS dans cette version, archive non vérifiée" >&2
fi
tar xzf "$archive" -C "$tmp"

# `easytab install` copie les programmes dans ~/.easytab/bin (easytab, easytab-term et, s'il est
# dans l'archive, easytab-overlay, la fenêtre flottante) et configure le shell.
if [ -n "${EASYTAB_SHELL:-}" ]; then
  "$tmp/easytab-$target/easytab" install --shell "$EASYTAB_SHELL"
else
  "$tmp/easytab-$target/easytab" install
fi
