PREFIX ?= $(HOME)/.local
PYTHON ?= /usr/bin/python3
DESKTOP_INTEGRATION ?= auto

.PHONY: build test test-clipboard test-ux test-languages showcase audit install desktop-install uninstall
build:
	cargo build --release --locked

test:
	cargo fmt --all -- --check
	$(PYTHON) tools/verify_backports.py
	cargo test --locked
	cargo clippy --all-targets --locked -- -D warnings
	$(PYTHON) -m unittest discover -s packaging/tests

test-clipboard:
	$(PYTHON) tools/test_clipboard.py

test-ux:
	$(PYTHON) tools/test_clipboard.py --suite editor
	$(PYTHON) tools/test_clipboard.py --suite layers

test-languages:
	@set -e; for language in en de fr es; do $(PYTHON) tools/test_clipboard.py --suite localization --language "$$language"; done

showcase:
	$(PYTHON) tools/render_showcase.py --record

audit:
	$(PYTHON) tools/verify_backports.py
	cargo test --release --locked --test security
	osv-scanner scan source --lockfile Cargo.lock

install: build
	$(PYTHON) packaging/install.py install --prefix "$(PREFIX)" --destdir "$(DESTDIR)" --desktop "$(DESKTOP_INTEGRATION)"

desktop-install:
	$(PYTHON) packaging/install.py desktop-install --prefix "$(PREFIX)"

uninstall:
	$(PYTHON) packaging/install.py uninstall --prefix "$(PREFIX)" --destdir "$(DESTDIR)" --desktop "$(DESKTOP_INTEGRATION)"
