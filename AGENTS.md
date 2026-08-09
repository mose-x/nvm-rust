# Agent Guidelines

## Code Hooks

This project uses an external code-hooks repository to enforce
commit/PR validation. Rules are defined in the public repo:

- Hook repo: `https://github.com/mose-x/code-hooks`

Key rules (full rules take precedence from the hook repo):

- Both commit author **and** committer emails must be on the allowlist (currently: `602187256@qq.com`)
- Total commit message length must NOT exceed 200 characters
- Forbidden tokens in commit messages: `Co-authored-by`, `traeagent`, etc.
- Use Conventional Commits format (e.g., `feat(install): ...`, `fix: ...`)
- Allowed push branch: `dev` only (main is protected, must go through PR)

## Unit Test Requirement

**All new code, bug fixes, and refactors MUST include unit tests. No exceptions.**

- Every new public function, bug fix, or behavior change requires at least one test.
- Tests should cover: the happy path, edge cases, and error paths.
- New modules must include a `#[cfg(test)]` block with at least basic coverage.
- If a change is difficult to test (e.g., network I/O), add tests for the testable
  parts (parsing, URL construction, logic branches) and mock/stub the rest.
- Shell script changes require integration tests that verify the script content
  (string-matching tests in `tests/` are acceptable for shell scripts).
- PRs without tests will NOT be merged — the reviewer should block on this.

### Platform-aware testing

Tests must pass on all 3 CI OSes (Ubuntu, Windows, macOS). When code is
platform-specific, guard tests with `#[cfg(unix)]` / `#[cfg(windows)]` /
`#[cfg(target_arch = "aarch64")]` so they only run on the relevant platform.

For shell script changes (`shell/nvm.sh`, `install.sh`, `install.ps1`), add
content-verification tests in `tests/` that read the file and assert the
expected strings are present (see `tests/p0_fixes.rs` for examples).

### Test isolation

**Never touch the real `~/.nvm.rust/` in tests.** Always set `NVM_DIR` to a
temp directory (`std::env::temp_dir()` in Rust, `%TEMP%\nvm-test-env` in
batch files). This prevents test interference with the user's actual nvm
installation and avoids flaky tests caused by stale state.

### Existing test locations

- Unit tests: `#[cfg(test)]` blocks inside each `src/*.rs` file (236 tests)
- Integration tests: `tests/*.rs` (94 tests across 8 files)
- Shell script verification: `tests/p0_fixes.rs` (7 tests)

## TODO Tracking

When a fix or feature addresses an item documented in a tracking file
(e.g. `note/todo.md`), the **same commit** that fixes the code MUST also
update the item's status in the tracking file (e.g. change "待处理" to
"✅ 已修复"). This keeps tracking files in sync with the codebase — no
stale "待处理" items for already-fixed bugs.

Rules:
- If the tracking file is in `.gitignore` (like `note/`), update it locally
  but do NOT `git add` it — the status is for local reference only.
- If the tracking file IS tracked by git, `git add` it together with the
  code fix so both land in the same commit.
- Update status to include the PR number or commit hash for traceability
  (e.g. `✅ 已修复 (PR #38)` or `✅ 已完成 (v2.1.0)`).

## Why Recreate `dev` Each Cycle

**Always create a fresh `dev` branch from `main` for each development cycle.
Never reuse an old `dev` or rebase `dev` onto `main`.**

Squash-merge commits on `main` have `committer = GitHub <noreply@github.com>`.
The pre-push hook rejects any noreply-committer commit inside `dev`'s push
range. If `dev` is rebased onto or merged with `main`, the noreply squash
commits enter `dev`'s push range and pre-push fails.

Creating `dev` fresh from `main` each cycle keeps the push range = only the
new commits (main's noreply squashes sit on `origin/main`, not counted as
"new" and not scanned), so pre-push passes.

## Caveats

- `git push --delete <branch>` and force-pushes are rejected by the pre-push
  hook (it errors on `new=0000` ref deletions and on noreply in a force-push
  range). Use the GitHub API (`DELETE /repos/.../git/refs/heads/dev`) or the
  GitHub UI "Delete branch" instead.
- Same applies to deleting a tag: use the GitHub API, not `git push --delete`.

## Development Workflow (Mandatory)

Every code change MUST follow this exact sequence. No steps may be skipped.

### Step 1: Branch
```bash
git checkout main && git pull origin main
git checkout -b dev   # only "dev" is allowed by pre-push hook
```

### Step 2: Code + Tests
- Write the fix/feature code.
- Write unit tests covering happy path + edge cases + error paths.
- If modifying shell scripts (`shell/nvm.sh`, `install.sh`, etc.), add
  content-verification tests in `tests/`.

### Step 3: Local Verification (MUST pass before commit)
Run ALL three checks. If ANY fails, fix the issue before proceeding.

```bash
# Set up environment (Windows with .svc Rust + MSVC Build Tools)
# See "Local Environment Setup" below for details.

cargo fmt          # auto-format
cargo fmt --check  # verify clean (exit 0 = pass)
cargo clippy --all-targets -- -D warnings   # zero warnings allowed
cargo test --all   # ALL tests must pass (unit + integration)
```

**If `cargo test --all` fails, do NOT proceed. Fix the failing test first.**
Do NOT use `--no-verify` to bypass hooks — fix the root cause.

### Step 4: Commit
```bash
git add <specific files>   # never `git add -A` or `git add .`
GIT_COMMITTER_NAME="mose-zm" GIT_COMMITTER_EMAIL="602187256@qq.com" \
git commit -m "fix(scope): description" --author="mose-zm <602187256@qq.com>"
```

The pre-commit hook will run `cargo fmt --check` and `cargo clippy -- -D warnings`
on staged Rust files. If the hook rejects, fix the issue and re-commit.

### Step 5: Push
```bash
# Delete old remote dev if it exists (from previous PRs):
curl -X DELETE -H "Authorization: token <TOKEN>" \
  https://api.github.com/repos/mose-x/nvm-rust/git/refs/heads/dev

# Push with proxy + SSL workaround (if behind Clash proxy):
GIT_SSL_NO_VERIFY=1 https_proxy=http://127.0.0.1:7890 \
git push -u origin dev
```

The pre-push hook will run `cargo test --all` (full test suite).
If any test fails, the push is rejected. Fix the test, re-commit, re-push.

### Step 6: PR + CI
```bash
# Create PR via GitHub API:
curl -X POST -H "Authorization: token <TOKEN>" \
  https://api.github.com/repos/mose-x/nvm-rust/pulls \
  -d '{"title":"...","head":"dev","base":"main","body":"..."}'
```

CI runs 5 checks on the PR:
1. `cargo fmt --check` (Linux)
2. `cargo clippy -- -D warnings` (Linux)
3. `cargo test --all` (Ubuntu)
4. `cargo test --all` (Windows)
5. `cargo test --all` (macOS)
6. commit-lint (PR only)

**ALL checks must pass (green). Do NOT merge if any check fails or is pending.**

### Step 7: Merge
```bash
curl -X PUT -H "Authorization: token <TOKEN>" \
  https://api.github.com/repos/mose-x/nvm-rust/pulls/<PR_NUMBER>/merge \
  -d '{"merge_method":"squash"}'
```

### Step 8: Cleanup
```bash
git checkout main
git pull origin main
git branch -d dev
```

### Step 9: Tag (only if releasing a new version)
```bash
GIT_COMMITTER_NAME="mose-zm" GIT_COMMITTER_EMAIL="602187256@qq.com" \
git tag -a v<X.Y.Z> -m "v<X.Y.Z>: description"
git push origin v<X.Y.Z>
```
Tagging triggers the release workflow (builds 8 platform binaries + publishes GitHub Release).
Only tag after all P0/P1 items in the current milestone are resolved.

## Local Environment Setup (Windows)

The pre-commit and pre-push hooks require a working Rust toolchain with MSVC.
On this machine, the `.svc` Rust installation has components in separate directories
that need to be added to PATH manually.

### Required batch file pattern (for hooks to pass):
```batch
@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
set RUST_HOME=C:\Users\mose\.svc\rust\1.93.1
set PATH=C:\Users\mose\.svc\nodejs\23.0.0;%RUST_HOME%\cargo\bin;%RUST_HOME%\rustc\bin;%RUST_HOME%\rustfmt-preview\bin;%RUST_HOME%\clippy-preview\bin;%PATH%
set NVM_DIR=%TEMP%\nvm-test-env
rd /s /q "%NVM_DIR%" 2>nul
mkdir "%NVM_DIR%"
```

### Known issues:
- `rustfmt.exe` needs `std-*.dll` and `rustc_driver-*.dll` from `rustc/bin/`
  (copy them to `rustfmt-preview/bin/` or add `rustc/bin` to PATH).
- `clippy-driver.exe` is in `clippy-preview/bin/` (not merged into `rustc/bin/`).
- Git push through Clash proxy needs `GIT_SSL_NO_VERIFY=1` (schannel doesn't
  work with HTTP proxies for HTTPS — use this workaround).
- Pre-push hook runs `cargo test --all` — needs Node.js on PATH for the
  `corepack_status_system_arg` test (add `.svc/nodejs/23.0.0` to PATH).
- `NVM_DIR` must point to an empty temp dir to avoid test interference
  with the real `~/.nvm.rust/` directory.

### Bash-to-cmd bridge:
From Git Bash, run `cmd //c <batch_file>.bat` to execute commands in a
cmd environment with the MSVC variables set. This is required because
`vcvars64.bat` only works in cmd, not in bash.
