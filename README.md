# dxray

`dxray` is a read-only static analyzer for Windows game installations on Linux.
It inspects PE metadata, import tables and local launcher configuration without
running the game, Wine or Proton.

It can report:

- rendering APIs visible through PE imports and delay imports;
- shipped DLSS, FSR, XeSS and NVIDIA Streamline files;
- local DLL overrides beside a game executable;
- Steam and Heroic installations found from local metadata;
- whether Proton offers NVAPI to each Steam game: the build's policy, the
  game's launch options, and what its last launch recorded.

When static evidence is insufficient, `dxray` says so. It does not guess which
renderer a game selects at runtime, whether the game launches, or how it will
perform.

## Support

Version 0.2.0 supports desktop Linux. Steam and Heroic discovery read local
installation metadata.

Flatpak, Snap and Steam Deck are not supported in this release.

## Install

Either route below leaves two binaries:

```text
dxray      Command-line analyzer
dxray-tui  Interactive library browser
```

### From a release

Every release attaches two Linux x86-64 archives and a `SHA256SUMS` file. Take
the `musl` build unless you have a reason not to: it is statically linked, so
nothing about it depends on the host's glibc. The `gnu` build is dynamically
linked against the system C library.

```sh
version=0.2.0
target=x86_64-unknown-linux-musl
base=https://github.com/cadocruz/dxray/releases/download/v$version

curl -LO "$base/dxray-$version-$target.tar.gz"
curl -LO "$base/SHA256SUMS"
sha256sum --check --ignore-missing SHA256SUMS

tar -xzf "dxray-$version-$target.tar.gz"
install -Dm755 "dxray-$version-$target/dxray" \
               "dxray-$version-$target/dxray-tui" -t ~/.local/bin
```

Read the `sha256sum` line before running anything: it prints `OK` per archive
and exits non-zero if a download is not what the release says it is. The last
command assumes `~/.local/bin` is on your `PATH`.

The archive holds the two binaries, this README and the licence. There is no
installer and nothing to uninstall — the binaries are the whole program, and
deleting them removes it.

Only Linux x86-64 is built. Anything else, Windows included, builds from source.

### From source

The crates are not published to crates.io yet. Build with Rust 1.88 or newer
and Cargo.

```sh
git clone https://github.com/cadocruz/dxray.git
cd dxray
cargo install --path crates/dxray-cli --locked
cargo install --path crates/dxray-tui --locked
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

### Commands

| Command | What it does | Argument |
|---|---|---|
| `inspect` | Reads PE files, or scans directories for `.exe` and `.dll` | `PATH...` |
| `game` | Ranks the executables in an install directory and analyzes the best | `PATH...` |
| `installed` | Lists what every supported launcher declares | — |
| `steam` | Lists what Steam declares | — |
| `nvapi` | Reads a Proton build's static NVAPI policy | `PROTON_PATH` |
| `help` | Prints help for the program or for one command | `[COMMAND]` |

| Option | `inspect` | `game` | `installed` | `steam` | `nvapi` |
|---|:-:|:-:|:-:|:-:|:-:|
| `--json` | ✓ | ✓ | ✓ | ✓ | |
| `--view compact\|full` | ✓ | ✓ | ✓ | ✓ | |
| `-r`, `--recursive` | ✓ | | | | |
| `--appid ID`, repeatable | | | | | ✓ |

`--view` cannot be combined with `--json`. Invalid options exit 2 before
anything is scanned.

`dxray-tui` takes no subcommand and nothing but `--help` and `--version`.

### What the views show

`--view compact` gives static renderer evidence, features and the essential
caveats; `--view full` gives absolute paths, versions, imports and the
executable ranking. Compact output retains each input's path.

The default differs by command. Without `--view`, `inspect` and `game` use the
standard text layout, while `installed` and `steam` use the compact inventory
tree.

Direct paths do not provide launcher or Proton context, so those reports mark
the static NVAPI policy as not assessed.

Inventory reports are grouped as launcher, library and game. Each game header
keeps its launcher ID and a static renderer result; the rows below it carry the
path, selected executable, Proton/NVAPI policy and any caveats. Installations
read successfully but with no game evidence are grouped explicitly rather than
hidden. Paths inside a library are relative to that library; externally managed
paths remain absolute. Compact keeps the essential caveats visible; full adds
library and launcher roots, the complete executable ranking and static evidence.
For a Steam game, the NVAPI row applies its launch options and any
`user_settings.py` to the policy of the Proton build that last ran it, and
quotes the `use_nvapi` value that launch recorded. It is read from files, not
observed at runtime; Heroic games are reported as not applicable.
Notes and problems remain visible in every view, with the existing exit codes.

```sh
dxray installed
dxray installed --view full
dxray steam
dxray steam --view full
```

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

## Environment

Discovery looks in the conventional locations. These variables say where else
to look, and are the answer for an installation this tool would not find on its
own. Each takes a platform-native path list, separated by `:` on Linux.

| Variable | Effect |
|---|---|
| `DXRAY_STEAM_ROOT` | Steam roots to add, considered after the conventional ones |
| `DXRAY_HEROIC_CONFIG` | Heroic configuration directories to add |
| `DXRAY_CONTAINER_HOMES` | Distrobox homes to inspect, in place of the scan below |

Without `DXRAY_CONTAINER_HOMES`, `dxray` looks for a Distrobox home on mounted
volumes: for each mount under `/media`, `/run/media` or `/mnt` it tries
`data/distrobox` and `distrobox`, lists their direct children, and considers
each child along with its `home` and `root` subdirectories. That is the whole
of it — two fixed subdirectories per mount and one level of children, never a
filesystem search.

Setting `DXRAY_CONTAINER_HOMES` to an **empty value** disables that scan
instead of adding to it, which is the difference between a run that depends on
what happens to be mounted and one that does not.

Steam consults container homes only when no Steam was found on the host, so a
native installation stays authoritative. Heroic always includes them, because
each Heroic configuration is an independent source of records rather than
another spelling of one installation.

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
