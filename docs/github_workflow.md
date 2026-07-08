# GitHub Workflow Guide

This note is for future agents who need to work on `OQQWall_rust` in a controlled way.

## 1. SSH login

Use the WSL environment for GitHub SSH operations.

1. Start an SSH agent in WSL if needed.
2. Load the private key:
   ```bash
   ssh-add ~/.ssh/id_ed25519
   ```
3. Verify GitHub SSH access:
   ```bash
   ssh -T git@github.com
   ```

If the host key prompt appears, accept the GitHub host fingerprint before retrying.

## 2. Work in WSL

Do not build or push from the Windows checkout directly.

Recommended flow:

1. Sync the repo into the WSL filesystem.
2. Make changes in the WSL copy.
3. Run `cargo fmt`, `cargo test`, and `cargo build` in WSL.
4. Copy release artifacts back to the Windows side if needed.

## 3. Split work by feature

Keep each feature isolated.

1. Create a branch per feature.
2. Commit only the files that belong to that feature.
3. Use a Conventional Commit message, for example:
   ```text
   feat(webview): add custom notification templates
   ```
4. Push the feature branch to the fork or remote.
5. Open a pull request targeting `master`.

## 4. Pull request guidance

Each PR should include:

- A short summary of the feature.
- Tests or build commands you ran.
- Any config changes or migration notes.

## 5. Practical note

If SSH fails, check these first:

- Whether the correct key is loaded.
- Whether the public key is added to the GitHub account.
- Whether `~/.ssh/known_hosts` contains GitHub's host key.
