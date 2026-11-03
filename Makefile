# Build the Flectar mailer. Cargo options come from .cargo/config.toml
# (single build tree in ./target, jobs = 1). Override with e.g. `make JOBS=8`.

BIN      := flectar-mail
PREFIX   ?= /usr/local
DESTDIR  ?=
JOBS     ?=
CARGO    ?= cargo
CARGOFLAGS := $(if $(JOBS),-j $(JOBS),)

.PHONY: all build release debug run check test fmt clippy install uninstall clean help

all: release

help:
	@echo "make [release]  optimized build   -> target/release/$(BIN)"
	@echo "make debug      debug build       -> target/debug/$(BIN)"
	@echo "make run        run the debug build"
	@echo "make check      cargo check (workspace, with tests)"
	@echo "make test       run the tests"
	@echo "make fmt        format; make clippy  lint"
	@echo "make install    install to \$$(DESTDIR)\$$(PREFIX)/bin (PREFIX=$(PREFIX))"
	@echo "make clean      remove target/"

build release:
	$(CARGO) build --release --bin $(BIN) $(CARGOFLAGS)

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

clean:
	$(CARGO) clean
