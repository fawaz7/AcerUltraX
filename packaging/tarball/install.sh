#!/bin/sh
# AcerUltraX installer, no package manager needed.
#
#   ./install.sh               install for you (into ~/.local)
#   sudo ./install.sh --system install for everyone (into /usr/local)
#   ./install.sh --uninstall   remove it again (add --system if you installed it that way)
#
# Your settings and the files AcerUltraX downloaded from Acer stay in ~/.local/share/acerultrax and
# ~/.config/acerultrax; delete those folders too to remove everything.
set -eu
cd "$(dirname "$0")"
ID=io.github.fawaz7.AcerUltraX

MODE=user
UNINSTALL=0
for a in "$@"; do
    case "$a" in
        --system) MODE=system ;;
        --uninstall) UNINSTALL=1 ;;
        -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
        *) echo "unknown option: $a" >&2; exit 2 ;;
    esac
done
if [ "$MODE" = system ]; then PREFIX=/usr/local; else PREFIX="$HOME/.local"; fi
BIN="$PREFIX/bin"
SHARE="$PREFIX/share"

if [ "$UNINSTALL" = 1 ]; then
    if [ "$MODE" = user ] && [ -x "$BIN/acerultrax-cli" ]; then
        "$BIN/acerultrax-cli" autostart off || true
    fi
    rm -f "$BIN/acerultrax" "$BIN/acerultrax-daemon" "$BIN/acerultrax-cli"
    rm -f "$SHARE/applications/$ID.desktop"
    find "$SHARE/icons/hicolor" -name "$ID*" -delete 2>/dev/null || true
    CONF="${XDG_CONFIG_HOME:-$HOME/.config}"
    rm -f "$CONF/autostart/acerultrax-tray.desktop" "$CONF/autostart/acerultrax-daemon.desktop"
    # The service file autostart wrote for this install.
    if grep -qs "$BIN/acerultrax-daemon" "$CONF/systemd/user/acerultrax.service"; then
        rm -f "$CONF/systemd/user/acerultrax.service"
        systemctl --user daemon-reload 2>/dev/null || true
    fi
    echo "AcerUltraX removed. Your sound is back to normal."
    exit 0
fi

if [ "$(uname -m)" != x86_64 ]; then
    echo "AcerUltraX needs a 64-bit Intel/AMD (x86_64) laptop." >&2
    exit 1
fi
for b in acerultrax acerultrax-daemon acerultrax-cli; do
    install -Dm755 "bin/$b" "$BIN/$b"
done
(cd share && find icons -type f -exec install -Dm644 {} "$SHARE/{}" \;)
# The menu entry points at the installed program, wherever that is.
mkdir -p "$SHARE/applications"
sed "s|^Exec=acerultrax|Exec=$BIN/acerultrax|" "share/applications/$ID.desktop" > "$SHARE/applications/$ID.desktop"
gtk-update-icon-cache -q -t "$SHARE/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$SHARE/applications" 2>/dev/null || true

echo "AcerUltraX is installed in $PREFIX."
echo "Open AcerUltraX from your applications and click \"Download and set up\"."
if [ "$MODE" = user ]; then
    case ":$PATH:" in
        *":$BIN:"*) ;;
        *) echo "(The command-line tool is $BIN/acerultrax-cli; add $BIN to your PATH to type just acerultrax-cli.)" ;;
    esac
fi
