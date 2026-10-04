#!/bin/sh
# Installe la dernière version d'EasyTab (Linux, macOS) :
#   curl -fsSL https://github.com/AdamSellin/EasyTab/releases/latest/download/install.sh | sh
# Variables : EASYTAB_VERSION (ex. v0.2.0, défaut : la dernière), EASYTAB_SHELL (zsh ou bash).
set -eu

repo="AdamSellin/EasyTab"
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target="x86_64-unknown-linux-gnu" ;;
  Darwin-arm64) target="aarch64-apple-darwin" ;;
  Darwin-x86_64) target="x86_64-apple-darwin" ;;
  *) echo "easytab : système non pris en charge ($(uname -s) $(uname -m))" >&2; exit 1 ;;
esac

if [ -n "${EASYTAB_VERSION:-}" ]; then
  url="https://github.com/$repo/releases/download/$EASYTAB_VERSION/easytab-$target.tar.gz"
else
  url="https://github.com/$repo/releases/latest/download/easytab-$target.tar.gz"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
echo "Téléchargement de $url"
curl -fsSL "$url" -o "$tmp/easytab.tar.gz"
tar xzf "$tmp/easytab.tar.gz" -C "$tmp"

# `easytab install` copie les programmes dans ~/.easytab/bin et configure le shell.
if [ -n "${EASYTAB_SHELL:-}" ]; then
  "$tmp/easytab-$target/easytab" install --shell "$EASYTAB_SHELL"
else
  "$tmp/easytab-$target/easytab" install
fi
