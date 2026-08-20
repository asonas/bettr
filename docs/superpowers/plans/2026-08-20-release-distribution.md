# GitHub Releases Distribution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a tag-driven GitHub Releases pipeline for four native macOS/Linux targets with checksums, attestations, and documented fixed-version upgrade/rollback.

**Architecture:** Keep the Rust binary unchanged and add two GitHub Actions workflows: CI for validation and Release for tagged builds. A small POSIX packaging script owns archive naming and checksum creation so local verification and CI use the same behavior; README documents download, `gh attestation verify`, atomic replacement, and rollback.

**Tech Stack:** Rust/Cargo, POSIX shell, GitHub Actions, GitHub CLI, GitHub artifact attestations.

**Spec:** `docs/superpowers/specs/2026-08-20-release-distribution-design.md`

## Global Constraints

- Release targets are exactly `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, and `aarch64-apple-darwin`.
- Release tags use `v<semver>` and must match the `bettr` package version from `cargo metadata`.
- Build jobs never receive `contents: write`; only the release job may create or upload a GitHub Release.
- Every release archive has a SHA-256 sidecar and appears in the sorted `SHA256SUMS` manifest.
- Existing `cargo install --path .` and skill installer workflows remain supported.
- No automatic PATH mutation, signing key secret, or new runtime dependency is introduced.

### Task 1: Add the deterministic packaging and verification script

**Files:**
- Create: `scripts/package-release.sh`
- Create: `scripts/verify-release.sh`
- Create: `tests/release_scripts.sh`

**Interfaces:**
- `scripts/package-release.sh <version> <target> <binary> <output-dir>` creates `bettr-<version>-<target>.tar.gz` and its `.sha256` sidecar.
- `scripts/verify-release.sh <archive> <checksum-file> <expected-version>` verifies the archive checksum, required files, and `bettr --version` after extraction.

- [ ] **Step 1: Write the failing shell fixture test**

Create an executable test that builds a temporary fake `bettr` executable printing `bettr 9.9.9`, writes a temporary LICENSE, invokes the packaging script, removes the source fake binary, and invokes the verification script against the archive and sidecar. Assert the archive contains `bettr` and `LICENSE`, the sidecar has a 64-character hex digest, and the verification output is successful.

- [ ] **Step 2: Run the fixture test to verify it fails**

Run: `sh tests/release_scripts.sh`

Expected: FAIL because the packaging and verification scripts do not exist.

- [ ] **Step 3: Implement the packaging script**

Use `set -eu`, validate all four arguments and the regular executable input, create a temporary staging directory, copy the binary as `bettr` plus repository `LICENSE`, create the gzip archive with deterministic member names, and emit a sidecar containing `<digest>  <archive-basename>`. Use `sha256sum` when available and `shasum -a 256` on macOS.

- [ ] **Step 4: Implement the verification script**

Validate the checksum sidecar against the archive, extract into a temporary directory, require executable `bettr` and `LICENSE`, run `bettr --version`, and require the expected version string. Exit nonzero with a concise diagnostic for each failed check.

- [ ] **Step 5: Run the fixture test to verify it passes**

Run: `sh tests/release_scripts.sh`

Expected: PASS, including archive listing, checksum validation, and version validation.

- [ ] **Step 6: Commit the scripts and test**

Run: `git add scripts/package-release.sh scripts/verify-release.sh tests/release_scripts.sh && git commit -m "add release packaging verification scripts"`

### Task 2: Add continuous integration workflow

**Files:**
- Create: `.github/workflows/ci.yml`

**Interfaces:**
- Workflow triggers on pull requests and pushes to `main`.
- Job runs `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --locked`, and `cargo build --locked --release`.

- [ ] **Step 1: Write workflow contract checks**

Extend `tests/release_scripts.sh` or add a POSIX assertion block to require the workflow contains both triggers, all four validation commands, `--locked`, and no release write permission.

- [ ] **Step 2: Run the contract check to verify it fails**

Run: `sh tests/release_scripts.sh`

Expected: FAIL with the missing `.github/workflows/ci.yml` path.

- [ ] **Step 3: Implement CI workflow**

Use `actions/checkout@v4`, install the stable Rust toolchain with rustup, add `rustfmt` and `clippy`, set `permissions: contents: read`, and run the four commands in one job on `ubuntu-24.04`.

- [ ] **Step 4: Run the contract check and Rust tests**

Run: `sh tests/release_scripts.sh && mise exec -- cargo test --locked`

Expected: PASS.

- [ ] **Step 5: Commit the CI workflow**

Run: `git add .github/workflows/ci.yml tests/release_scripts.sh && git commit -m "add continuous integration workflow"`

### Task 3: Add tag-driven release workflow

**Files:**
- Create: `.github/workflows/release.yml`

**Interfaces:**
- Tag push `v*` starts the workflow.
- Matrix build jobs use the exact target/runner pairs from the spec, upload archives, and create attestations.
- A dependent release job creates a GitHub Release with all archives, sidecars, and `SHA256SUMS`.

- [ ] **Step 1: Add release workflow contract assertions**

Require the YAML to contain the tag trigger, all four target strings, all four runner labels, `cargo metadata`, `--locked`, the packaging and verification scripts, `actions/attest@v4`, `actions/upload-artifact@v4`, `contents: write` only in the release job, and `--verify-tag` on release creation.

- [ ] **Step 2: Run the assertions to verify they fail**

Run: `sh tests/release_scripts.sh`

Expected: FAIL because `.github/workflows/release.yml` does not exist.

- [ ] **Step 3: Implement the matrix build jobs**

Define a four-entry matrix with `target` and `runs-on`. For each entry, install the target, compare `${GITHUB_REF_NAME#v}` with the package version from `cargo metadata --no-deps --format-version 1`, build with `cargo build --locked --release --target`, invoke the target-native binary with `--version`, package it, verify it, upload the archive and sidecar, and attest the archive with the least permissions required.

- [ ] **Step 4: Implement the release aggregation job**

Download all build artifacts into one directory, concatenate and sort sidecars into `SHA256SUMS`, verify every archive again, and run `gh release create "$GITHUB_REF_NAME" --verify-tag --generate-notes --repo "$GITHUB_REPOSITORY"` with `GH_TOKEN` and `contents: write`. Do not delete or overwrite an existing release.

- [ ] **Step 5: Run workflow contract checks**

Run: `sh tests/release_scripts.sh`

Expected: PASS for all required release workflow strings and permission boundaries.

- [ ] **Step 6: Commit the release workflow**

Run: `git add .github/workflows/release.yml tests/release_scripts.sh && git commit -m "add tagged binary release workflow"`

### Task 4: Document installation and recovery

**Files:**
- Modify: `README.md`
- Modify: `docs/implementation-roadmap.md`

**Interfaces:**
- README documents a concrete versioned Release URL format, checksum verification, attestation verification, upgrade, rollback, and retained `cargo install`/skill installer paths.
- Roadmap records that GitHub Releases distribution is implemented and that platform-specific installer packages remain out of scope.

- [ ] **Step 1: Add documentation assertions**

Require README to mention `SHA256SUMS`, `gh attestation verify`, a versioned `/releases/download/v...` URL, a `.prev` rollback path, `cargo install --path .`, and the skill installer command.

- [ ] **Step 2: Run assertions to verify they fail**

Run: `sh tests/release_scripts.sh`

Expected: FAIL because the new release instructions are absent.

- [ ] **Step 3: Write the fixed-version procedure**

Document selecting the target archive, downloading the archive and `SHA256SUMS` for the same tag, checking `sha256sum -c` or `shasum -a 256`, optionally running `gh attestation verify`, extracting to a temporary directory, replacing the installed binary only after `--version` succeeds, and restoring `.prev` if the new binary fails.

- [ ] **Step 4: Document compatibility boundaries**

Keep `cargo install --path .` for local development and retain the existing Codex skill installer command. State that Release archives are the supported short install path, while Homebrew, OS packages, automatic updates, and key-based developer signing are not part of this Issue.

- [ ] **Step 5: Run documentation and full focused verification**

Run: `sh tests/release_scripts.sh && mise exec -- cargo fmt --all -- --check && mise exec -- cargo clippy --all-targets --all-features -- -D warnings && mise exec -- cargo test --locked && mise exec -- cargo build --locked --release`

Expected: PASS; record the existing `cli_latency` benchmark separately if it still fails its pre-existing assertion.

- [ ] **Step 6: Commit documentation**

Run: `git add README.md docs/implementation-roadmap.md tests/release_scripts.sh && git commit -m "document binary release operations"`

### Task 5: Final review and Issue evidence

**Files:**
- Modify: `bettr#14` conversation only

- [ ] **Step 1: Inspect the final diff and workflow permissions**

Run `git diff main...HEAD --check`, inspect both workflow files, and confirm no credentials, signing keys, raw command secrets, or unrelated files are included.

- [ ] **Step 2: Run the final verification commands**

Run the focused verification command from Task 4 and record exact results, including test count and release build status.

- [ ] **Step 3: Record the implementation and verification in bettr#14**

Add one `[Conversation update]` comment with the implemented workflow, artifact names, verification commands, and any intentionally deferred packaging. Only after the merged `main` has the same commit and verification evidence should the Issue transition to `done`.

- [ ] **Step 4: Prepare local merge**

Re-read `bettr#14`, verify its latest revision, merge the verified branch into `main`, rerun the focused verification on merged `main`, then clean up only this worktree and branch.
