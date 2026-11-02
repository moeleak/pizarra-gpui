# Contributing

## Development environment

The flake provides everything needed to build and run the app:

```sh
direnv allow     # with direnv: loads the dev shell from .envrc (`use flake`)
nix develop      # without direnv
```

Before sending a change, make sure these pass:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
nix flake check  # also builds the package
```

## Commit messages

**All commits must follow [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/).**

```
<type>[optional scope]: <description>

[optional body]

[optional footer(s)]
```

- `type` is one of:

  | type | use for |
  | --- | --- |
  | `feat` | a new user-visible feature |
  | `fix` | a bug fix |
  | `perf` | a performance improvement |
  | `refactor` | code changes that neither fix a bug nor add a feature |
  | `docs` | documentation only |
  | `test` | adding or fixing tests |
  | `build` | Cargo, dependencies, the Nix flake/package |
  | `ci` | CI configuration |
  | `style` | formatting, no code change |
  | `chore` | anything else (tooling, repo housekeeping) |
  | `revert` | reverting a previous commit |

- `scope` is optional and names the area touched, e.g. `canvas`, `ui`,
  `export`, `config`, `geometry`, `nix`, `deps`.
- The description is imperative, lower-case, without a trailing period, and
  the whole header stays within 72 characters.
- Breaking changes add `!` after the type/scope and a `BREAKING CHANGE:`
  footer explaining the migration.

Examples:

```
feat(ui): add opacity slider to the header bar
fix(canvas): keep strokes visible when zoomed past 64x
build(nix): add rust-analyzer to the dev shell
docs: document the configuration file
```
