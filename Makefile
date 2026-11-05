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

.PHONY: all build release fast run-fast debug run check test fmt clippy install uninstall sweep clean help

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
	@echo "make sweep      delete build files unused for $(SWEEP_DAYS) days (SWEEP_DAYS=n, DRY=1 to only list)"
	@echo "make clean      remove target/"

build release:
	$(CARGO) build --release --bin $(BIN) $(CARGOFLAGS)

# Nearly as fast to run as release, much faster to build: no LTO and
# parallel code generation. For daily use; release stays the distributed build.
fast:
	$(CARGO) build --profile fast --bin $(BIN) $(CARGOFLAGS)

run-fast:
	$(CARGO) run --profile fast --bin $(BIN) $(CARGOFLAGS)

debug:
	$(CARGO) build --bin $(BIN) $(CARGOFLAGS)

run:
	$(CARGO) run --bin $(BIN) $(CARGOFLAGS)

check:
	$(CARGO) check --workspace --tests $(CARGOFLAGS)

test:
	$(CARGO) test --workspace $(CARGOFLAGS)

fmt:
	$(CARGO) fmt --all

clippy:
	$(CARGO) clippy --workspace --tests $(CARGOFLAGS)

install: release
	install -Dm755 target/release/$(BIN) $(DESTDIR)$(PREFIX)/bin/$(BIN)

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/$(BIN)

# Builds keep every older compiled version in target/, which grows to tens of
# gigabytes. Needs cargo-sweep: `cargo install cargo-sweep`.
sweep:
	@command -v cargo-sweep >/dev/null || { echo "cargo-sweep is missing: cargo install cargo-sweep" >&2; exit 1; }
	$(CARGO) sweep --time $(SWEEP_DAYS) $(SWEEPFLAGS)
	$(CARGO) sweep --installed $(SWEEPFLAGS)

clean:
	$(CARGO) clean
