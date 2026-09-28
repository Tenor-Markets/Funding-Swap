# Protocol

Lean Solana math primitives and the production Anchor program scaffold.

The workspace is intentionally narrow:

- `crates/fixed-perp-math` holds the checked fixed-point primitives.
- `programs/fixed-perp` holds the production Anchor program scaffold.
- `guide/guide.md` is reconciled with the implementation; its `DONT TOUCH`
  suffix remains byte-for-byte protected.

## Pinned environment

- Host Rust is selected locally by `rust-toolchain.toml` at `1.90.0`.
- The Rust packages retain `rust-version = "1.89"` because Solana CLI 3.1.15 builds SBF with its Rust 1.89 compiler.
- Anchor CLI and the `anchor-lang` and `anchor-client` crates are pinned to `1.0.2`.
- Solana CLI is pinned to `3.1.15`.
- Cargo selects `/usr/bin/gcc` for the x86_64 host linker through `.cargo/config.toml`.

The workspace pin does not change the global rustup default. Opening this directory causes rustup and rust-analyzer to select Rust 1.90.0 for this workspace.

## Build and test

From this directory, run the plain repository commands without shell environment overrides:

```bash
anchor build
cargo test -p fixed-perp-math
cargo clippy --workspace --all-targets -- -D warnings
```

## VS Code on WSL

Install the WSL extension once, then open the pinned workspace:

```bash
code --install-extension rust-lang.rust-analyzer
code /home/orinayo/dev/solana/fixed-perp/protocol
```

If this directory was already open before the toolchain pin changed, run `Developer: Reload Window` in VS Code.
