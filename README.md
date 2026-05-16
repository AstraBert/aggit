# aggit

S3-backed, git-versioned object storage for agents.

`aggit` is a Rust CLI that implements a minimal Git-like content store in `.aggit/`, plus remote sync to S3-compatible storage.

> [!NOTE]
>
> _This software is still in alpha: you might encounter bugs and breaking changes might be introduced in future versions_

## Installation

```bash
cargo install aggit@0.2.0-alpha
```


> [!NOTE]
>
> `aggit` is not yet compatible with Windows.

### As an Agent Skill

You can use `aggit` as an agent skill, downloading it with the `skills` CLI tool:

```bash
npx skills add AstraBert/aggit
```

Or copy-pasting the [`SKILL.md`](./skills/aggit/SKILL.md) file to your own skills setup.

## What It Does

- Initializes an `aggit` repository with internal refs and object storage.
- Hashes file contents into compressed objects (`blob`, `tree`, `commit`) under `.aggit/objects`.
- Tracks staged files in a branch-specific index (`.aggit/refs/index/<branch>`).
- Supports core local VCS-like commands:
  - `add`, `commit`, `checkout`, `status`, `diff`, `ls`, `cat`
  - `switch` (branch switching/creation)
  - `branch` (list local branches)
- Stores global commit author config at `~/.config/.aggit/author.toml`.
- Stores repository metadata in `.aggit/repo.toml`.
- Manages multiple S3 origins in `.aggitorigin` (`create`, `add`, `update`).
- Pushes reachable local objects and branch metadata to S3.
- Clones `.aggit` data from S3 and restores the working tree.

> [!NOTE]
>
> _`gitops` implementation is heavilys inspired by [`pygit`](https://github.com/benhoyt/pygit/blob/master/pygit.py)_

## CLI Commands

- `aggit init <path>`: initialize repository at `path`
- `aggit author -n <name> -e <email>`: set global author
- `aggit repo <name> [-d <description>] [-t <topic> ...]`: configure repository metadata
- `aggit add <files...>`: stage files
- `aggit commit -m <message>`: commit staged state
- `aggit checkout <commit_sha>`: restore working tree/index to a specific commit (commit-only checkout)
- `aggit status`: show changed/new/deleted files vs index
- `aggit diff`: show unified diff of working copy vs index
- `aggit ls [-d]`: list index entries (`-d` for mode/SHA/stage)
- `aggit cat -m <mode> -s <sha1-prefix>`: inspect objects (`blob|tree|commit|type|size|pretty`)
- `aggit switch <branch> [-c]`: switch branch, optionally create (branch switching is done with `switch`, not `checkout`)
- `aggit branch`: list branches
- `aggit origin <create|add|update> <name> -e <endpoint> -s <secret> -k <key_id> -r <region>`
- `aggit push <origin>`: push current branch objects/head/index to remote bucket
- `aggit clone -o <origin> -n <repo_name> [-b <branch>]`: clone from remote bucket

## Remote Storage Model

When pushing, `aggit` targets bucket:

- `<origin>-<repository-name>-<branch>`

It uploads:

- reachable objects from local head (excluding what remote head already has)
- branch head file
- branch index file
- remote `head` key

Clone downloads remote `.aggit` data, sets `.aggit/HEAD`, then restores files into the working tree.

## Quick Start

```bash
# Build
cargo build --release

# Init repo
aggit init .

# Configure identity + repository metadata
aggit author -n "Jane Doe" -e "jane@example.com"
aggit repo my-repo -d "Agent artifacts" -t agents -t storage

# Local workflow
echo "hello" > notes.txt
aggit add notes.txt
aggit commit -m "initial commit"
aggit status

# Configure first S3 origin
aggit origin create prod \
  -e https://s3.example.com \
  -s <SECRET_KEY> \
  -k <KEY_ID> \
  -r us-east-1

# Push
aggit push prod
```

## License

This project is provided under MIT license.
