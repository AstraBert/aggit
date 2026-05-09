---
name: aggit
description: Use this skill to persist intermediate changes and artifacts in a S3-backed, git-like system without polluting the git history in the current repository
compatibility: Requires the aggit CLI (installable via 'cargo install aggit')
license: MIT
metadata:
  author: Clelia Astra Bertelli
  version: 0.1.0
---

# aggit skill

## Purpose

Use `aggit` to checkpoint agent work products (draft files, generated assets, experiment outputs) into a separate, S3-backed object history.

This is useful when you want:
- reversible checkpoints during long-running work
- branchable storage for artifacts
- remote persistence without committing to the repository's Git history

## When to use

Use this skill when the user asks to:
- save intermediate outputs outside normal Git commits
- back up generated artifacts to S3-compatible storage
- branch/switch artifact snapshots independently from repository Git branches
- restore a prior artifact state from remote object storage

Avoid this skill when the user explicitly wants standard Git commits/PR workflows.

## Prerequisites

- `aggit` available on PATH
- project initialized as an aggit repo (`.aggit/` exists), or permission to initialize it
- S3-compatible endpoint and credentials for push/clone
- repository metadata configured via `aggit repo`

## One-time setup

```bash
# In repository root
aggit init .
aggit author -n "<NAME>" -e "<EMAIL>"
aggit repo <repo_name> -d "<description>" -t <topic>

# Create first remote origin
aggit origin create <origin_name> \
  -e <s3_endpoint> \
  -s <secret_key> \
  -k <key_id> \
  -r <region>
```

Notes:
- Origin data is stored in `.aggitorigin`.
- `.aggitorigin` is auto-added to `.gitignore` and `.aggitignore`.

## Core workflow

### Save a checkpoint

```bash
aggit add <file1> <file2> ...
aggit commit -m "checkpoint: <what changed>"
```

### Inspect state

```bash
aggit status
aggit diff
aggit ls -d
```

### Branch artifact history

```bash
aggit switch <branch> -c   # create
aggit switch <branch>      # switch existing
aggit branch
```

### Push to remote

```bash
aggit push <origin_name>
```

### Clone/restore from remote

```bash
aggit clone -o <origin_name> -n <repo_name> -b <branch>
```

## Operational guidance

- Keep commit messages explicit and task-scoped (`checkpoint: parsed invoices v3`).
- Prefer frequent small commits over large snapshots.
- Run `aggit status` before `switch`; branch switching requires a clean working tree.
- Use dedicated aggit branch names per experiment (`exp-parser-a`, `exp-parser-b`).
- Push after meaningful milestones to avoid local-only history.

## Safety rules

- Never place credentials in user-facing logs or summaries.
- Treat `.aggitorigin` as sensitive local config.
- Do not overwrite origin settings unless explicitly requested.
- If clone/restore would overwrite expected local work, confirm intent first.

## Troubleshooting

- `Repository is yet to be configured`:
  run `aggit repo <name> ...`.
- `The commit author should be globally configured`:
  run `aggit author -n ... -e ...`.
- `No current head, nothing to push`:
  create at least one commit before push.
- `Current working tree has uncommitted changes` on switch:
  commit or stash changes first (with aggit commits).
- `No such origin`:
  verify `.aggitorigin` entry or add/update origin.

## Expected outcomes

After using this skill, artifact state should be:
- versioned locally in `.aggit` as object/commit history
- optionally synchronized to S3-compatible remote storage
- restorable by branch via `clone`/`switch` + working-tree restoration
