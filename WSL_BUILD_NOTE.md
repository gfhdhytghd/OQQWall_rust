# WSL Build Note

## For Later Agents

On this machine, do **not** treat Windows PowerShell as the primary Rust build environment for `cargo check` / `cargo build`.

Use this flow instead:

1. Copy the repo into the **WSL Linux filesystem**.
   Example:
   ```bash
   mkdir -p ~/work
   rsync -a --delete /mnt/c/Users/Administrator/Downloads/oqqwall-plus/OQQWall_rust/ ~/work/OQQWall_rust/
   ```

2. Enter the WSL copy and run Cargo **inside WSL**.
   Example:
   ```bash
   cd ~/work/OQQWall_rust
   cargo check -p OQQWall_RUST
   cargo build --release -p OQQWall_RUST
   ```

3. After the build finishes, copy the outputs back to the Windows side.
   Example:
   ```bash
   mkdir -p /mnt/c/Users/Administrator/Downloads/oqqwall-plus/OQQWall_rust/artifacts
   cp target/release/OQQWall_RUST /mnt/c/Users/Administrator/Downloads/oqqwall-plus/OQQWall_rust/artifacts/
   ```

## Why

- Windows-side Rust builds here may hit toolchain / linker issues.
- Building directly under `/mnt/c/...` is slower and more fragile than building under `~/...` inside WSL.
- The stable path is:
  - sync code into WSL
  - run Cargo in WSL
  - copy artifacts back to Windows

## Practical Rule

If a later task needs Rust compilation, default to:

- source code in WSL
- `cargo` in WSL
- result artifacts copied back to `C:\...` / `/mnt/c/...`
