# Build Evanya. Cargo options come from .cargo/config.toml
# (single build tree in ./target, jobs = 8). Override with e.g. `make JOBS=4`.

BIN      := evanya
PREFIX   ?= /usr/local
DESTDIR  ?=
JOBS     ?=
CARGO    ?= cargo
CARGOFLAGS := $(if $(JOBS),-j $(JOBS),)
# `make sweep` keeps build files used in the last SWEEP_DAYS days.
SWEEP_DAYS ?= 14
SWEEPFLAGS := $(if $(DRY),--dry-run,)
# PDF previews need the PDFium library beside the executable. The pinned,
# checksum-verified build is downloaded once and kept in target/pdfium-cache.
PDFIUM_TARGET ?= linux-x64
PDFIUM_LIB    := libpdfium.so
stage_pdfium = test -f $(1)/$(PDFIUM_LIB) || python3 scripts/stage-pdfium.py $(PDFIUM_TARGET) $(1) --cache target/pdfium-cache

.PHONY: all build release fast run-fast debug run check test fmt clippy install uninstall sweep pdfium clean help

all: release

help:
	@echo "make [release]  optimized build   -> target/release/$(BIN)"
	@echo "make fast       quick optimized   -> target/fast/$(BIN) (no LTO, parallel)"
	@echo "make run-fast   run the fast build"
	@echo "make debug      debug build       -> target/debug/$(BIN)"
	@echo "make run        run the debug build"
	@echo "make check      cargo check (workspace, with tests)"
	@echo "make test       run the tests"
	@echo "make fmt        format; make clippy  lint"
	@echo "make install    install to \$$(DESTDIR)\$$(PREFIX)/bin (PREFIX=$(PREFIX))"
	@echo "make pdfium     put PDFium (for PDF previews) beside every build"
	@echo "make sweep      delete build files unused for $(SWEEP_DAYS) days (SWEEP_DAYS=n, DRY=1 to only list)"
	@echo "make clean      remove target/"

build release:
	$(CARGO) build --release --bin $(BIN) $(CARGOFLAGS)
	@$(call stage_pdfium,target/release)

# Nearly as fast to run as release, much faster to build: no LTO and
# parallel code generation. For daily use; release stays the distributed build.
fast:
	$(CARGO) build --profile fast --bin $(BIN) $(CARGOFLAGS)
	@$(call stage_pdfium,target/fast)

run-fast: fast
	target/fast/$(BIN)

debug:
	$(CARGO) build --bin $(BIN) $(CARGOFLAGS)
	@$(call stage_pdfium,target/debug)

run: debug
	target/debug/$(BIN)

pdfium:
	@for dir in target/release target/fast target/debug; do \
		mkdir -p $$dir && { $(call stage_pdfium,$$dir); }; \
	done

check:
	$(CARGO) check --workspace --tests $(CARGOFLAGS)

test:
	$(CARGO) test --workspace $(CARGOFLAGS)

fmt:
	$(CARGO) fmt --all

clippy:
	$(CARGO) clippy --workspace --tests $(CARGOFLAGS)

# PDFium goes to $(PREFIX)/lib/evanya, where the application looks for it.
install: release
	install -Dm755 target/release/$(BIN) $(DESTDIR)$(PREFIX)/bin/$(BIN)
	install -Dm755 target/release/$(PDFIUM_LIB) $(DESTDIR)$(PREFIX)/lib/evanya/$(PDFIUM_LIB)
	mkdir -p $(DESTDIR)$(PREFIX)/lib/evanya/pdfium-licenses
	cp -r target/release/pdfium-licenses/. $(DESTDIR)$(PREFIX)/lib/evanya/pdfium-licenses/

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/$(BIN)
	rm -rf $(DESTDIR)$(PREFIX)/lib/evanya

# Builds keep every older compiled version in target/, which grows to tens of
# gigabytes. Needs cargo-sweep: `cargo install cargo-sweep`.
sweep:
	@command -v cargo-sweep >/dev/null || { echo "cargo-sweep is missing: cargo install cargo-sweep" >&2; exit 1; }
	$(CARGO) sweep --time $(SWEEP_DAYS) $(SWEEPFLAGS)
	$(CARGO) sweep --installed $(SWEEPFLAGS)

clean:
	$(CARGO) clean
