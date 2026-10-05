#!/bin/sh
# Installe la dernière version d'EasyTab (Linux, macOS) :
#   curl -fsSL https://github.com/AdamSellin/EasyTab/releases/latest/download/install.sh | sh
# Dépôt privé : depuis un clone du dépôt, `sh scripts/install.sh`. Le script se
# sert alors des identifiants GitHub de git (ou de $GITHUB_TOKEN, ou de gh).
# Variables : EASYTAB_VERSION (ex. v0.2.0, défaut : la dernière), EASYTAB_SHELL (zsh ou bash).
set -eu

repo="AdamSellin/EasyTab"
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target="x86_64-unknown-linux-gnu" ;;
  Darwin-arm64) target="aarch64-apple-darwin" ;;
  Darwin-x86_64) target="x86_64-apple-darwin" ;;
  *) echo "easytab : système non pris en charge ($(uname -s) $(uname -m))" >&2; exit 1 ;;
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
private_download() {
  token="${GITHUB_TOKEN:-}"
  if [ -z "$token" ] && command -v git >/dev/null 2>&1; then
    token="$(printf 'protocol=https\nhost=github.com\n\n' \
      | GIT_TERMINAL_PROMPT=0 git credential fill 2>/dev/null | sed -n 's/^password=//p')"
  fi
  [ -n "$token" ] || return 1
  asset_url="$(curl -fsSL -H "Authorization: Bearer $token" \
      "https://api.github.com/repos/$repo/releases/$release" \
    | tr -d '\n' | sed 's/}, *{/}\n{/g' | grep "\"name\": *\"$asset\"" \
    | sed -n 's/.*"url": *"\(https:[^"]*\/releases\/assets\/[0-9]*\)".*/\1/p' | head -n 1)"
  [ -n "$asset_url" ] || return 1
  curl -fsSL -H "Authorization: Bearer $token" -H "Accept: application/octet-stream" \
    "$asset_url" -o "$archive"
}

echo "Téléchargement de $url"
if ! curl -fsSL "$url" -o "$archive" 2>/dev/null; then
  echo "Lien direct indisponible (dépôt privé ?), téléchargement avec tes identifiants GitHub"
  if ! private_download; then
    if command -v gh >/dev/null 2>&1; then
      gh release download ${EASYTAB_VERSION:-} -R "$repo" -p "$asset" -O "$archive"
    else
      echo "easytab : téléchargement impossible. Dépôt privé : connecte git à GitHub, ou définis GITHUB_TOKEN." >&2
      exit 1
    fi
  fi
fi
tar xzf "$archive" -C "$tmp"

# `easytab install` copie les programmes dans ~/.easytab/bin (easytab, easytab-term et, s'il est
# dans l'archive, easytab-overlay, la fenêtre flottante) et configure le shell.
if [ -n "${EASYTAB_SHELL:-}" ]; then
  "$tmp/easytab-$target/easytab" install --shell "$EASYTAB_SHELL"
else
  "$tmp/easytab-$target/easytab" install
fi
