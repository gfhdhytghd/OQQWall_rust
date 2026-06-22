# Repository Guidelines

## Project Structure & Module Organization
- `crates/app`: Binary entry point and wiring (config loading, runtime bootstrap).
- `crates/core`: Functional core (events, state, reducers, deciders) with most logic and tests.
- `crates/drivers`: IO drivers for NapCat WS and Qzone sending.
- `crates/infra`: Persistence primitives (journal, snapshot, blob).
- `docs/`: Design, runbooks, and configuration references.
- `res/`: Rendering resources and assets used by the renderer.
- `data/`: Runtime data (snapshots, journals). Keep out of version control.

## Build, Test, and Development Commands
- `cargo run -p OQQWall_RUST -- oobe`: Generate a starter `config.json`.
- `cargo run -p OQQWall_RUST`: Run the service locally (reads `./config.json` by default).
- `cargo test`: Run all tests across the workspace.
- `cargo test -p core`: Focus on functional core correctness.
- `cargo fmt --check`: Enforce formatting.
- `cargo clippy -D warnings`: Run linting as errors.
- `cargo check`: Run after each change to catch compile errors early.

## Coding Style & Naming Conventions
- Rustfmt is the source of truth for formatting; use 4-space indentation.
- Follow Rust naming: `snake_case` for modules/functions, `CamelCase` for types, `SCREAMING_SNAKE_CASE` for constants.
- Keep pure logic in `crates/core` and IO in `crates/drivers` to preserve the Functional Core / Imperative Shell split.

## Testing Guidelines
- Core tests live in `crates/core/tests` (for example, `reduce_replay.rs`, `decide_tick.rs`).
- Prioritize reducer replay, decider tick behavior, and command parsing stability.
- Run `cargo test -p core` before touching IO-heavy paths.

## Commit & Pull Request Guidelines
- Use Conventional Commits for commit messages: `type(scope): subject`.
- Recommended types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`.
- Keep subject concise and action-oriented (prefer lowercase, no trailing period).
- Examples:
  - `feat(telemetry): add batched sample uploader`
  - `fix(web_api): stabilize rendered post tests`
  - `docs(config): document telemetry options`
- PRs should include a clear description, the tests you ran, and note any config or behavior changes (especially `config.json`/`devconfig.json`).

## Configuration & Runtime Notes
- Use `OQQWALL_NAPCAT_TOKEN` to avoid committing access tokens.
- `devconfig.json` is read only in debug builds; release ignores it.
- Debug builds mirror stderr logs to `data/logs/debug.log` (base `OQQWALL_DATA_DIR`), override with `OQQWALL_DEBUG_LOG`.
- Persistent data and snapshots belong under `data/`; do not commit generated files in `target/`.

## Container Build Environment
- On this machine, prefer **WSL Linux filesystem builds** over Windows-side `cargo`.
- Read `WSL_BUILD_NOTE.md` before running Rust compilation commands from a Windows-hosted checkout.
- Use `Dockerfile.rust-glibc231-toolchain` to build a fixed toolchain image based on `rust-glibc231:20.04`.
- The image includes required build deps: `python3`, `pkg-config`, `libfreetype6-dev`, `libfontconfig1-dev`, `ca-certificates`.
- `cargo build`, `cargo test`, and `cargo check` should all run in this container environment for consistency.
- Build image:
  - `docker build --network host -t rust-glibc231:20.04-oqqwall -f Dockerfile.rust-glibc231-toolchain .`
- Run build container:
  - `docker run --rm -it --network host -v "$HOME/data:/data" -w /data -v "$HOME/.cargo/registry:/root/.cargo/registry" -v "$HOME/.cargo/git:/root/.cargo/git" -v "$PWD/target:/work/target" rust-glibc231:20.04-oqqwall bash`
- In container:
  - `cd /data/OQQWall_rust && cargo check`
  - `cd /data/OQQWall_rust && cargo test`
  - `cd /data/OQQWall_rust && cargo build -r`
