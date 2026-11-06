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

## Forked dependencies

`Cargo.toml` pulls two forks over git (sibling checkouts live in
`../pizarra` and `../gpui`):

- `moeleak/pizarra` (`main`) — engine with per-segment pressure widths and
  the `*_pressure` input methods. Keep the old pointer methods working for
  callers that have no pressure.
- `moeleak/zed` (`gpui-v0.2.2-tablet`) — GPUI 0.2.2 with `PointerType` and
  `pressure` on mouse events, fed by XInput on X11, `tablet-unstable-v2` on
  Wayland and the tablet subtype of AppKit mouse events on macOS; Windows
  reports mouse events without pressure.

To pick up a change, push it to the fork, then `cargo update -p pizarra` or
`cargo update -p gpui` (never a blanket update) and refresh the matching
`outputHashes` entry in `nix/package.nix`. When either API changes, update the
matching README section about tablets.
