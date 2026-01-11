CARGO ?= cargo
DOCKER ?= docker
INSTALL_ROOT ?= $(HOME)/.local
BUNDLES_DIR ?= $(CURDIR)/bundles
ARCHES ?= linux/amd64 linux/arm64
OPENSSH_VERSION ?= 9.7p1
BUNDLE_VERSION ?= $(shell sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)+bundle1
BUNDLE_FILES := $(foreach arch,$(ARCHES),$(BUNDLES_DIR)/openssh-bundle-$(arch).tar.xz)

.PHONY: all build install fmt check clean bundles

all: build

build:
	$(CARGO) build --release

install: bundles
	$(CARGO) install --path . --locked --root $(INSTALL_ROOT)
	@mkdir -p $(INSTALL_ROOT)/bundles
	@(cd $(BUNDLES_DIR) && tar cf - .) | (cd $(INSTALL_ROOT)/bundles && tar xf -)

fmt:
	$(CARGO) fmt

check:
	$(CARGO) check

clean:
	$(CARGO) clean

bundles: $(BUNDLE_FILES)

$(BUNDLES_DIR)/openssh-bundle-%.tar.xz: Dockerfile.bundle
	@mkdir -p $(dir $@)
	@set -euo pipefail; \
	ARCH="$*"; \
	TAG="$${ARCH//\//-}"; \
	BUNDLE_FILE="$(notdir $@)"; \
	echo "Building bundle $$BUNDLE_FILE for $$ARCH"; \
	DOCKER_BUILDKIT=1 $(DOCKER) build --platform $$ARCH \
		--build-arg OPENSSH_VERSION=$(OPENSSH_VERSION) \
		--build-arg BUNDLE_VERSION=$(BUNDLE_VERSION) \
		--build-arg BUNDLE_FILENAME=$$BUNDLE_FILE \
		-t sshpod-bundle-$$TAG \
		-f Dockerfile.bundle .; \
	CID="$$( $(DOCKER) create sshpod-bundle-$$TAG )"; \
	$(DOCKER) cp $$CID:/out/$$BUNDLE_FILE "$@"; \
	$(DOCKER) rm $$CID >/dev/null
