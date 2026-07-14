#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(grep -Po '(?<=^version = ")[^"]+' Cargo.toml | head -1)
echo "==> Building CSSwitch Desktop v${VERSION}"

# Desktop 独立构建
cd desktop/src-tauri && cargo build --release && cd ../../

BIN="desktop/src-tauri/target/release/desktop"
[ -f "$BIN" ] || { echo "ERROR: desktop binary not found at $BIN"; exit 1; }
strip "$BIN" 2>/dev/null || true
chmod 755 "$BIN"

rm -rf pkg releases/csswitch-desktop_*_amd64.deb
mkdir -p pkg/csswitch-desktop_${VERSION}/usr/local/bin \
         pkg/csswitch-desktop_${VERSION}/usr/share/applications \
         pkg/csswitch-desktop_${VERSION}/usr/share/icons/hicolor/32x32/apps \
         pkg/csswitch-desktop_${VERSION}/usr/share/icons/hicolor/64x64/apps \
         pkg/csswitch-desktop_${VERSION}/usr/share/icons/hicolor/128x128/apps \
         pkg/csswitch-desktop_${VERSION}/usr/share/icons/hicolor/256x256/apps \
         pkg/csswitch-desktop_${VERSION}/DEBIAN \
         releases

cp "$BIN" pkg/csswitch-desktop_${VERSION}/usr/local/bin/csswitch-desktop
cp desktop/src-tauri/icons/32x32.png       pkg/csswitch-desktop_${VERSION}/usr/share/icons/hicolor/32x32/apps/csswitch.png
cp desktop/src-tauri/icons/64x64.png       pkg/csswitch-desktop_${VERSION}/usr/share/icons/hicolor/64x64/apps/csswitch.png
cp desktop/src-tauri/icons/128x128.png     pkg/csswitch-desktop_${VERSION}/usr/share/icons/hicolor/128x128/apps/csswitch.png
cp desktop/src-tauri/icons/128x128@2x.png  pkg/csswitch-desktop_${VERSION}/usr/share/icons/hicolor/256x256/apps/csswitch.png
cp csswitch.desktop pkg/csswitch-desktop_${VERSION}/usr/share/applications/

cat > pkg/csswitch-desktop_${VERSION}/DEBIAN/control << EOF
Package: csswitch-desktop
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: amd64
Depends: csswitch (= ${VERSION}), libc6 (>= 2.28), libssl3 | libssl1.1, libwebkit2gtk-4.1-0 | libwebkit2gtk-4.0-37, libgtk-3-0, libayatana-appindicator3-1 | libappindicator3-1
Maintainer: Yuntao <YuntaoOvO@github.com>
Homepage: https://github.com/YuntaoOvO/CSSwitch-Linux
Description: CSSwitch desktop GUI with system tray for Linux
 A Tauri-based desktop app that remotely controls the CSSwitch daemon.
 Close to tray, right-click to stop daemon and quit.
 Requires the csswitch CLI package.
EOF

cat > pkg/csswitch-desktop_${VERSION}/DEBIAN/postinst << 'ENDSCRIPT'
#!/bin/sh
set -e
echo "CSSwitch Desktop v${VERSION} installed."
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications >/dev/null 2>&1 || true
fi
echo "Launch from app menu or run: csswitch-desktop"
ENDSCRIPT
sed -i "s/\${VERSION}/${VERSION}/g" pkg/csswitch-desktop_${VERSION}/DEBIAN/postinst

chmod 755 pkg/csswitch-desktop_${VERSION}/DEBIAN pkg/csswitch-desktop_${VERSION}/DEBIAN/postinst
dpkg-deb --root-owner-group --build pkg/csswitch-desktop_${VERSION} releases/csswitch-desktop_${VERSION}_amd64.deb
rm -rf pkg

echo "==> releases/csswitch-desktop_${VERSION}_amd64.deb"
