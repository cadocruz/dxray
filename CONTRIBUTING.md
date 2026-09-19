# Contributing to dxray

Issues and pull requests are welcome. The project is a static analyzer, so a
change must make its evidence and its limits clearer, not turn a weak signal
into a confident verdict.

## Before opening an issue

Search existing issues first. For a detection or discovery problem, include:

- dxray version and Linux distribution;
- the command you ran and its exit code;
- relevant text or JSON output;
- what you expected and what happened.

Do not attach proprietary game executables, Proton prefixes, full Steam or
Heroic configuration files, screenshots with personal paths, tokens or account
information. Replace local paths with neutral examples before posting output.

## Pull requests

Keep each pull request focused. Explain what static evidence changed, what the
tool now reports, and which false-positive or false-negative risk the change
introduces or avoids.

Run these checks before sending a pull request:

```sh
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

New parsing or discovery behavior needs tests. A real-machine report is useful,
but it does not replace a reproducible fixture.

## Scope

dxray does not run games, Wine or Proton. Contributions should preserve that
boundary unless the maintainers explicitly change the project scope.

By submitting a contribution, you agree to license it under Apache-2.0.
