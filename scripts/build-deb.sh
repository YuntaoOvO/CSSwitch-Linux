#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(grep -Po '(?<=^version = ")[^"]+' Cargo.toml | head -1)
echo "==> Building CSSwitch CLI v${VERSION}"

cargo build --release -p csswitch -p csswitch-gateway
strip target/release/csswitch target/release/csswitch-gateway
chmod 755 target/release/csswitch target/release/csswitch-gateway

rm -rf pkg releases/csswitch_*_amd64.deb
mkdir -p pkg/csswitch_${VERSION}/usr/local/bin pkg/csswitch_${VERSION}/DEBIAN releases

cp target/release/csswitch target/release/csswitch-gateway pkg/csswitch_${VERSION}/usr/local/bin/

cat > pkg/csswitch_${VERSION}/DEBIAN/control << EOF
Package: csswitch
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: amd64
Depends: libc6 (>= 2.28), libssl3 | libssl1.1
Maintainer: Yuntao <YuntaoOvO@github.com>
Homepage: https://github.com/YuntaoOvO/CSSwitch-Linux
Description: Proxy launcher for Claude Science on Linux
 CSSwitch Linux is a proxy launcher for Claude Science on Linux, WSL,
 and headless environments.
EOF

cat > pkg/csswitch_${VERSION}/DEBIAN/postinst << 'ENDSCRIPT'
#!/bin/sh
set -e
echo "CSSwitch Linux v${VERSION} installed. Docs: https://github.com/YuntaoOvO/CSSwitch-Linux"
ENDSCRIPT
# 用 sed 替换模板里的变量（heredoc 里没法直接用 ${VERSION}）
sed -i "s/\${VERSION}/${VERSION}/g" pkg/csswitch_${VERSION}/DEBIAN/postinst

chmod 755 pkg/csswitch_${VERSION}/DEBIAN pkg/csswitch_${VERSION}/DEBIAN/postinst
dpkg-deb --root-owner-group --build pkg/csswitch_${VERSION} releases/csswitch_${VERSION}_amd64.deb
rm -rf pkg

echo "==> releases/csswitch_${VERSION}_amd64.deb"
