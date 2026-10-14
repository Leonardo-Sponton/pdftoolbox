#!/bin/sh
# Instala o PDFToolbox para o usuário atual (padrão: ~/.local), sem sudo.
#   ./install.sh               compila em modo release e instala
#   ./install.sh --uninstall   remove os arquivos instalados
#   PREFIX=/usr/local ./install.sh   instala em outro prefixo (pode exigir sudo)
set -eu

APP_ID=io.github.leonardo_sponton.PDFToolbox
PREFIX="${PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin/pdftoolbox"
DESKTOP="$PREFIX/share/applications/$APP_ID.desktop"
ICON="$PREFIX/share/icons/hicolor/scalable/apps/$APP_ID.svg"
cd "$(dirname "$0")"

refresh() {
    command -v update-desktop-database >/dev/null && update-desktop-database -q "$PREFIX/share/applications" || true
    command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true
}

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$BIN" "$DESKTOP" "$ICON"
    refresh
    echo "PDFToolbox removido de $PREFIX."
    echo "Preferências (~/.config/pdftoolbox) e cache (~/.cache/pdftoolbox) foram mantidos; apague-os se quiser."
    exit 0
fi

cargo build --release --locked
install -Dm755 target/release/pdftoolbox "$BIN"
install -Dm644 "data/$APP_ID.desktop" "$DESKTOP"
install -Dm644 "data/icons/$APP_ID.svg" "$ICON"
refresh
echo "Instalado em $PREFIX. Abra “PDFToolbox” no menu de aplicativos ou rode: pdftoolbox"
for t in qpdf pdftoppm gs; do
    command -v "$t" >/dev/null || echo "Aviso: '$t' não encontrado. Veja o README (Dependências)."
done
