# Agent Guidelines

## Code Hooks

本项目使用外部 code-hooks 进行 commit/PR 校验，规则定义在公共仓库：

- Hook 仓库地址：`https://github.com/mose-x/code-hooks`

关键规则摘要（完整规则以 hook 仓库为准）：

- Commit author 和 committer 邮箱必须在 allowlist 内（当前可用：`602187256@qq.com`）
- Commit message 总长度不超过 200 字符
- 禁止在 commit message 中出现 `Co-authored-by`、`traeagent` 等 token
- 使用 Conventional Commits 格式（如 `feat(install): ...`、`fix: ...`）

## 提交流程

- **不要直推 main**。main 是保护分支，需走 feature 分支 + PR。
- 流程：
  1. 新建 feature 分支：`git checkout -b feat/xxx`
  2. 提交时用 `GIT_COMMITTER_NAME` / `GIT_COMMITTER_EMAIL` 和 `--author` 同时指定 author 与 committer 为 allowlist 内的邮箱
  3. 推送 feature 分支并创建 PR
  4. 推送成功后等待 CI 全绿
  5. CI 全绿后再 merge 到 main
