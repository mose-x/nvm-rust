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

## Unit Test Requirement

**All new code, bug fixes, and refactors MUST include unit tests.**

- Every new public function, bug fix, or behavior change requires at least one test.
- Tests should cover: the happy path, edge cases, and error paths.
- New modules must include a `#[cfg(test)]` block with at least basic coverage.
- If a change is difficult to test (e.g., network I/O), add tests for the testable
  parts (parsing, URL construction, logic branches) and mock/stub the rest.
- PRs without tests will NOT be merged — the reviewer should block on this.

## Commit Workflow

- **Do NOT push directly to `main`**. `main` is a protected branch; always go through a feature branch + PR.
- Workflow:
  1. Create a feature branch: `git checkout -b feat/xxx`
  2. Commit with both `GIT_COMMITTER_NAME` / `GIT_COMMITTER_EMAIL` and `--author` so that BOTH author and committer use an allowlisted email
  3. Push the feature branch and create a PR
  4. After push succeeds, wait for all CI checks to go green
  5. Merge to main only after all CI passes
