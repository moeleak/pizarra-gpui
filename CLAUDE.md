# Pizarra (GPUI)

GPUI frontend for the `pizarra` drawing engine, ported from pizarra-gtk.
See README.md for the architecture and CONTRIBUTING.md for the rules.

## Rules

- **Commits must use Conventional Commits** (`feat:`, `fix:`, `build(nix):`,
  ...). See CONTRIBUTING.md for the allowed types, scopes and format.
- Keep user-facing texts in English.
- Before committing: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`; for changes to Cargo.toml/Cargo.lock/nix files also `nix flake check`.

## Environment

- Dev shell: `direnv allow` (`.envrc` has `use flake`) or `nix develop`.
- `Cargo.lock` is seeded from gpui 0.2.2's published lockfile; newer `libc`
  breaks gpui's `xattr` 0.2.3 dependency, so don't blanket `cargo update`.
