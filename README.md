# dxray

`dxray` is a read-only static analyzer for Windows game installations on Linux.
It inspects PE metadata, import tables and local launcher configuration without
running the game, Wine or Proton.

It can report:

- rendering APIs visible through PE imports and delay imports;
- shipped DLSS, FSR, XeSS and NVIDIA Streamline files;
- local DLL overrides beside a game executable;
- Steam and Heroic installations found from local metadata;
- Proton NVAPI policy as written by the selected Proton build.

When static evidence is insufficient, `dxray` says so. It does not guess which
renderer a game selects at runtime, whether the game launches, or how it will
perform.

## Support

Version 0.0.1 supports desktop Linux. Steam and Heroic discovery read local
installation metadata.

Flatpak, Snap and Steam Deck are not supported in this release.

## Install

`dxray` requires Rust 1.88 or newer and Cargo.

```sh
git clone https://github.com/cadocruz/dxray.git
cd dxray
cargo install --path crates/dxray-cli --locked
cargo install --path crates/dxray-tui --locked
```

The commands install two binaries:

```text
dxray      Command-line analyzer
dxray-tui  Interactive library browser
```

To build without installing them:

```sh
cargo build --release
```

The binaries are written to `target/release/`.

## Use

Inspect one PE image:

```sh
dxray inspect path/to/game.exe
```

Rank executables in a game directory and analyze the best candidate:

```sh
dxray game path/to/game-directory
```

List local Steam and Heroic installations:

```sh
dxray installed
```

List Steam only:

```sh
dxray steam
```

Inspect the NVAPI policy in a Proton installation:

```sh
dxray nvapi path/to/Proton --appid 1088850
```

Start the terminal browser:

```sh
dxray-tui
```

For `inspect` and `game`, use `--view compact` for static renderer evidence,
features and essential caveats, or `--view full` for absolute paths, versions,
imports and executable ranking. Compact output retains each input's path.
`--view` cannot be combined with `--json`. Only `inspect` accepts
`-r` / `--recursive`; only `nvapi` accepts repeatable `--appid ID` options.
Without `--view`, reports use the standard text layout. Invalid
options exit 2 before scanning. Direct paths do not provide launcher
or Proton context, so these reports mark the static NVAPI policy as not assessed.

Use `--json` with `inspect`, `game`, `installed` or `steam` when another tool
will consume the output. `nvapi` intentionally has no JSON format yet.

```bash
dxray inspect --view compact path/to/game.exe path/to/another.exe
dxray inspect --recursive --json path/to/library
dxray game --view full path/to/game-directory
dxray installed --json
dxray nvapi path/to/Proton --appid 1088850 --appid 570
```

`inspect` and `game` require one or more `PATH` arguments. `nvapi` requires
one `PROTON_PATH`, either an installation directory or its `proton` script.
Options belong after the subcommand and may appear before or after paths.
Use `--` before a path beginning with a hyphen. Run `dxray COMMAND --help`
for command-specific options.

## Exit codes

| Code | Meaning |
|---:|---|
| 0 | The requested inspection completed. |
| 1 | A file could not be read, a scan reached a safety limit, or launcher discovery could not completely answer the request. |
| 2 | The command line is invalid. |

For an inventory scan, exit code 1 means the output may be incomplete. Read the
diagnostics or the JSON summary before treating an inventory as exhaustive.

## What dxray does not do

`dxray` does not execute inspected binaries, install or modify game files,
start Wine or Proton, contact online services, collect telemetry, or measure
performance. A local DLL or an API name in a file is evidence about disk
contents, not proof that the game loads it at runtime.

## Project layout

| Crate | Purpose |
|---|---|
| `dxray-pe` | Dependency-free PE parsing: machine type, imports and version resources. |
| `dxray-core` | Analysis, executable ranking, Steam/Heroic discovery and Proton policy reading. |
| `dxray-cli` | The `dxray` command-line program. |
| `dxray-tui` | The separate interactive terminal browser. |

Keeping the TUI separate means scripts that install `dxray` do not also pull in
terminal UI dependencies.

## Development

Run the checks before proposing a change:

```sh
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for reporting and contribution guidance.

## License

Apache-2.0. See [LICENSE](LICENSE).
