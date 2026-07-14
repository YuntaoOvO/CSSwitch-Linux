.PHONY: all build clean install deb rpm

PREFIX ?= /usr/local
VERSION := 1.1.1

all: build

build:
	cargo build --release -p csswitch -p csswitch-gateway

clean:
	cargo clean
	rm -rf releases/

install: build
	install -Dm755 target/release/csswitch $(DESTDIR)$(PREFIX)/bin/csswitch
	install -Dm755 target/release/csswitch-gateway $(DESTDIR)$(PREFIX)/bin/csswitch-gateway

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/csswitch $(DESTDIR)$(PREFIX)/bin/csswitch-gateway

deb: build
	strip target/release/csswitch target/release/csswitch-gateway
	mkdir -p pkg/csswitch_$(VERSION)/usr/local/bin pkg/csswitch_$(VERSION)/DEBIAN
	cp target/release/csswitch target/release/csswitch-gateway pkg/csswitch_$(VERSION)/usr/local/bin/
	cat > pkg/csswitch_$(VERSION)/DEBIAN/control << 'EOF'
Package: csswitch
Version: $(VERSION)
Section: utils
Priority: optional
Architecture: amd64
Depends: libc6 (>= 2.28), libssl3 | libssl1.1
Maintainer: Yuntao <YuntaoOvO@github.com>
Homepage: https://github.com/YuntaoOvO/CSSwitch-Linux
Description: Provider switcher and launcher for Claude Science on Linux
 CSSwitch Linux is a pure CLI port of CSSwitch v0.4.4 for Linux, WSL,
 and headless environments.
EOF
	cat > pkg/csswitch_$(VERSION)/DEBIAN/postinst << 'ENDSCRIPT'
#!/bin/sh
set -e
echo "CSSwitch Linux v$(VERSION) installed. Docs: https://github.com/YuntaoOvO/CSSwitch-Linux"
ENDSCRIPT
	chmod 755 pkg/csswitch_$(VERSION)/DEBIAN pkg/csswitch_$(VERSION)/DEBIAN/postinst
	dpkg-deb --build pkg/csswitch_$(VERSION) releases/csswitch_$(VERSION)_amd64.deb
	rm -rf pkg
	@echo "→ releases/csswitch_$(VERSION)_amd64.deb"

rpm: build
	strip target/release/csswitch target/release/csswitch-gateway
	@echo "rpmbuild not available in this environment."
	@echo "On a Red Hat / Fedora system:"
	@echo "  sudo dnf install rpm-build"
	@echo "  make rpm"
	@echo ""
	@echo "Or convert the deb:"
	@echo "  sudo apt install alien"
	@echo "  alien --to-rpm releases/csswitch_$(VERSION)_amd64.deb"
	@if command -v rpmbuild >/dev/null 2>&1; then \
		mkdir -p rpmbuild/{BUILD,RPMS,SOURCES,SPECS} && \
		mkdir -p rpmbuild/BUILD/csswitch-$(VERSION)/usr/local/bin && \
		cp target/release/csswitch target/release/csswitch-gateway rpmbuild/BUILD/csswitch-$(VERSION)/usr/local/bin/ && \
		cp pkg/csswitch.spec rpmbuild/SPECS/ && \
		rpmbuild -bb --define "_topdir $$(pwd)/rpmbuild" rpmbuild/SPECS/csswitch.spec && \
		cp rpmbuild/RPMS/x86_64/*.rpm releases/ && \
		rm -rf rpmbuild && \
		echo "→ releases/csswitch-$(VERSION)-1.x86_64.rpm"; \
	fi
