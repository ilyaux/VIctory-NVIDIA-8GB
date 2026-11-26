# Shared checkout coordination

Read `docs/asset-policy.md` before working. Game files and derived content stay
outside Git; only source, documentation and our own synthetic fixtures belong here.

- Use this checkout. Do not create another project copy, branch or worktree for
  Codex/Claude cooperation unless the user changes that instruction.
- Coordinate in Kanzo project `victory`. Codex owns the parent task `victory#655`
  and dispatches scoped tasks to Claude Code. Read your task and its latest comments
  first; do not take unrelated tasks or read other projects.
- Before editing, record the files you own and the intended checks in your task.
  Change only those files. Request a handoff through Kanzo before touching another
  worker's files; a board comment is communication, not an atomic file lock.
- Preserve existing changes. Do not switch branches, reset, stash, revert or unstage
  another worker's work. Codex reviews the combined diff and performs commit/push.
- Codex controls game launches, GPU/WARP tests, profilers and shared build windows.
  Claude runs compilation/tests only when given an explicit command and time window.
  Reading and analysis can continue while a native test runs.
- Report source facts, hypotheses, executed checks and untested cases separately.
  Synthetic success is not correct gameplay or stable 30 FPS. Return finished work
  to `review`; leave `done` to the user.
- Keep credentials and local authentication/configuration out of commits and reports.
  Do not enable REA unless the user explicitly requests it.
