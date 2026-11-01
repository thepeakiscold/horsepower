# HorsePower — small, honest helpers for humans.

PYTHON ?= python3
DEB_VERSION ?= 1.0.0
ARCH := $(shell dpkg --print-architecture 2>/dev/null || echo amd64)
DEB := dist/horsepower_$(DEB_VERSION)_$(ARCH).deb
CARGO ?= $(shell command -v cargo 2>/dev/null || echo $(HOME)/.cargo/bin/cargo)
RUST := rust/horsepowerd

.PHONY: all test test-rust test-python lint lint-rust deb build install run preview clean

all: test deb

# Everything the package ships is exercised by one of these.
test: test-rust test-python

test-rust:
	cd $(RUST) && $(CARGO) test --release --locked

test-python:
	PYTHONPATH=src $(PYTHON) -m unittest discover -s tests -v

lint: lint-rust
	$(PYTHON) -m pyflakes src scripts/*.py tests bin/horsepower

# Needs the rustfmt and clippy components:
#   rustup component add rustfmt clippy
lint-rust:
	cd $(RUST) && $(CARGO) fmt --check && $(CARGO) clippy --all-targets -- -D warnings

deb:
	$(PYTHON) scripts/build-deb.py

install: $(DEB)
	sudo apt-get install -y ./$(DEB)

# A debhelper build, for uploads and PPAs. Produces the .deb one directory up.
build:
	dpkg-buildpackage -b -us -uc

# Make the build work without network access (needed on Launchpad).
vendor:
	$(PYTHON) scripts/vendor-rust.py

run:
	$(PYTHON) bin/horsepower

preview:
	GDK_BACKEND=x11 $(PYTHON) scripts/preview.py preview.png

clean:
	rm -rf dist debian/horsepower .mypy_cache .ruff_cache $(RUST)/target
	find . -name '__pycache__' -type d -prune -exec rm -rf {} +
	find . -name '*.pyc' -delete
