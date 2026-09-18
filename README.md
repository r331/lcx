# lcx — LeetCode in your terminal

[![CI](https://github.com/HarryYCChou/lcx/actions/workflows/ci.yml/badge.svg)](https://github.com/HarryYCChou/lcx/actions)
[![Release](https://img.shields.io/github/v/release/HarryYCChou/lcx)](https://github.com/HarryYCChou/lcx/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`lcx` lets you browse, read, solve, test, and submit LeetCode problems without
leaving your terminal. It talks to the live LeetCode API (GraphQL + judge
endpoints) and caches the problem list locally for instant browsing.

Run it with no arguments for a full **interactive TUI**, or use the individual
subcommands for scripting and one-offs. The core (HTTP client, cache, config) is
deliberately separated from the presentation layer, so the CLI and TUI share the
same LeetCode logic.

## Demo

![lcx demo](docs/lcx.gif)

## Screenshots

**Login modal** — auto-detects browser cookies:

![Login modal](docs/screenshots/login.png)

**Main dashboard** — menu, profile stats, and problem search:

![Main dashboard](docs/screenshots/home.png)

**Solve view** — description, solution preview, and test/submit output:

![Solve view](docs/screenshots/solve.png)

## Installation

Prebuilt binaries are attached to the [latest release](https://github.com/HarryYCChou/lcx/releases/latest).

### Linux

```bash
# x86_64
curl -LO https://github.com/HarryYCChou/lcx/releases/latest/download/lcx-x86_64-unknown-linux-gnu.tar.gz
tar -xzf lcx-x86_64-unknown-linux-gnu.tar.gz
install -m 755 lcx ~/.local/bin/lcx   # or somewhere on your PATH
```

### macOS

```bash
# Apple Silicon
curl -LO https://github.com/HarryYCChou/lcx/releases/latest/download/lcx-aarch64-apple-darwin.tar.gz
tar -xzf lcx-aarch64-apple-darwin.tar.gz

# universal (works with or without Homebrew)
sudo mkdir -p /usr/local/bin
sudo install -m 755 lcx /usr/local/bin/lcx

# or, if you use Homebrew (already on PATH, no sudo needed):
install -m 755 lcx /opt/homebrew/bin/lcx
```

The binary is unsigned, so macOS may quarantine it. If it refuses to run:
`xattr -d com.apple.quarantine "$(command -v lcx)"`.

### Windows

Download and extract the zip, then put `lcx.exe` somewhere on your `PATH`
(PowerShell):

```powershell
Invoke-WebRequest -Uri https://github.com/HarryYCChou/lcx/releases/latest/download/lcx-x86_64-pc-windows-msvc.zip -OutFile lcx.zip
Expand-Archive lcx.zip -DestinationPath .
```

### From source

Requires a recent Rust toolchain ([rustup](https://rustup.rs/)).

```bash
git clone https://github.com/HarryYCChou/lcx.git
cd lcx
cargo build --release
# binary at ./target/release/lcx
```

Copy `target/release/lcx` somewhere on your `PATH` (e.g. `~/.local/bin`).

## Quick Start

`lcx` is TUI-first — the whole flow (log in, search, solve) happens in the app:

```bash
# 1. Populate the local problem cache
lcx cache --update

# 2. Launch the interactive TUI
lcx

# From here everything happens inside the TUI:
# 3. Log in           <F1> opens the login modal; it auto-detects your browser
#                     cookies (<F3> opens the LeetCode login page), then <Enter>
#                     manual login? see "Finding your session cookies" below
# 4. Set language     first time only: <Tab> to the menu, pick "Set default
#                     language", <Enter> to choose (used for new solution files)
# 5. Pick a problem   <Tab> back to search, type to filter, <↑>/<↓> move, <Enter> open
# 6. Write code       <e> (or <Enter>) opens the file in $EDITOR; <r> starts over
# 7. Test & submit    <t> runs the sample cases, <s> submits for a verdict
#                     <Esc> goes back to the list (<Esc> again quits)
```

> **Windows:** cookie auto-detect with Chrome/Edge/Brave needs an admin
> PowerShell — see [Authentication](#authentication) for details.

### TUI at a glance

- **Login modal** — hands-free cookie auto-detection (`F3` open login page, `F2`
  toggle auto-detect, `F4` browse offline, `Esc` cancel). Function keys are used
  because multiplexers like tmux reserve the Ctrl prefix.
- **Main dashboard** — `Tab` switches focus between the action menu (set language,
  login, delete cache, reset config, quit) and the problem search; `F5` refresh,
  `F1` login, `Esc` quit.
- **Solve view** — left: description with LeetCode's inline markup preserved;
  right: read-only solution preview. `e`/`Enter` edit in `$EDITOR`, `r` start over,
  `Tab` cycle panes, `j`/`k` scroll, `t` test, `s` submit, `Esc` back.

## Commands

| Command | Description |
| --- | --- |
| `lcx` | Launch the interactive TUI (default) |
| `lcx login [--session <s>] [--csrf <c>]` | Save session credentials (prompts if omitted) |
| `lcx whoami` | Show the currently authenticated user |
| `lcx list [--difficulty] [--tag] [--status] [--query] [--limit]` | List cached problems with filters |
| `lcx show <id\|slug> [--lang] [--code]` | Show a problem's description (optionally starter code) |
| `lcx pick <id\|slug> [--lang] [--no-open]` | Generate a solution file and open it |
| `lcx edit <id\|slug> [--lang] [--no-open]` | Reopen (or generate) a solution file |
| `lcx test <id\|slug\|path> [--case] [--lang]` | Run a solution against sample/custom cases |
| `lcx submit <id\|slug\|path> [--lang]` | Submit a solution and print the verdict |
| `lcx daily [--pick]` | Show today's daily challenge (optionally scaffold it) |
| `lcx cache [--update] [--clear]` | Manage the local problem cache |
| `lcx config [set <key> <value>]` | View or change configuration (`lang`, `editor`, `workspace`) |
| `lcx sets <list|create|import|add|remove|delete>` | Manage curated and custom problem sets |

### Problem sets

The dashboard has a **Problem Sets** tile between Menu and Profile. Press `Tab` to focus it, select **NeetCode 150** or **Amazon**, then choose **All categories** or a category such as **Two Pointers** or **Stack**. The problem list shows each problem's difficulty and category. Both sets are bundled for offline browsing; problem status and IDs come from the local LeetCode cache when available. Run `lcx cache --update` to refresh that cache.

Create a company or personal set with the CLI. It appears in the tile the next time you launch `lcx`:

```sh
lcx sets create "Meta"
lcx sets add "Meta" two-sum --category "Arrays & Hashing"
lcx sets list
```

For a larger list, create a UTF-8 text file with one `slug,category` pair per line, then import it. LeetCode problem URLs work in place of slugs. A `slug,category` header and lines beginning with `#` are optional.

```text
slug,category
two-sum,Arrays & Hashing
valid-parentheses,Stack
```

```sh
lcx sets import "My Company" company-problems.txt
```

Custom sets live in the app's `sets.json` file alongside its config. NeetCode 150 is sourced from [NeetCode's public problem metadata](https://github.com/neetcode-gh/leetcode/blob/main/.problemSiteData.json). Amazon is a community [three-month company-tag snapshot](https://github.com/liquidslr/leetcode-company-wise-problems/blob/03850eb5d16892514491cf1381c32ec0330a2719/Amazon/2.%20Three%20Months.csv) from August 16, 2026; categories use NeetCode patterns where available and LeetCode topics for the rest.

### Authentication

LeetCode has no official API, so `lcx` uses the same session cookies your browser
does.

- **Easiest:** launch `lcx` and let the login modal **auto-detect** cookies from a
  browser you're already signed into (press `F3` to open the login page first).
  The modal also imports `cf_clearance` when that browser has one.
  On **Windows**, Chrome/Edge/Brave encrypt cookies with app-bound encryption, so
  auto-detect can only read them if you run `lcx` as administrator — right-click
  PowerShell -> "Run as administrator" (or run `Start-Process powershell -Verb
  RunAs`), then run `lcx`. Firefox and the manual method below work without admin.
- **Manual:** copy your `LEETCODE_SESSION` and `csrftoken` cookies (see
  [Finding your session cookies](#finding-your-session-cookies)), then:

```bash
lcx login --session "<LEETCODE_SESSION>" --csrf "<csrftoken>"
# or just `lcx login` to be prompted
# if Cloudflare clearance is needed after login:
lcx login --cf-clearance "<cf_clearance>"
```

`cf_clearance` is optional. You can also enter it in the TUI login modal, or
update only that cookie with `lcx login --cf-clearance "<value>"` when your
session and CSRF cookies are already saved. Credentials are stored in the
configuration file shown by `lcx config`, with `600` permissions on Unix.
Verify with `lcx whoami`.

### Finding your session cookies

You only need this for manual login (the TUI can auto-detect cookies for you).

1. Open [leetcode.com](https://leetcode.com) in your browser and **sign in**.
2. Open your browser's developer tools:
   - **Chrome / Edge / Brave:** `F12` (or `Ctrl+Shift+I`, `Cmd+Option+I` on macOS)
     → **Application** tab → **Storage** → **Cookies** → `https://leetcode.com`.
   - **Firefox:** `F12` → **Storage** tab → **Cookies** → `https://leetcode.com`.
3. Find the session and CSRF rows and copy each **Value**:
   - `LEETCODE_SESSION` — a long token (this is your session).
   - `csrftoken` — a shorter token.
   - `cf_clearance` — optional Cloudflare clearance after a browser challenge.
4. Pass them to `lcx login` as shown above.

Treat these like passwords. They expire periodically, so if requests start
failing, refresh them or run `lcx login` again.

### Advanced: work from the command line

Prefer to skip the TUI? The whole solve loop is available as plain commands:

```bash
lcx login                    # sign in (see Authentication / Finding your session cookies)
lcx list --difficulty easy   # browse problems
lcx pick 1 --lang rust       # scaffold a solution file and open your editor
lcx test 1                   # run against sample cases
lcx submit 1                 # submit for a verdict
```

More examples:

```bash
lcx list --tag array --status todo --query "two sum" --limit 20
lcx show two-sum --code
lcx test ~/lcx/1.two-sum.rs
lcx test 1 --case $'[2,7,11,15]\n9'
lcx daily --pick
lcx config set editor "code -w"
```

Solution files are written to the workspace directory as `{id}.{slug}.{ext}`;
`test` and `submit` identify the problem from the filename. Config and cache
live in the app's configuration directory (run `lcx config` to see its path),
and solutions default to `~/lcx/`.

### Supported languages

`rust`, `python3`, `cpp`, `c`, `java`, `csharp`, `javascript`, `typescript`,
`go`/`golang`, `ruby`, `swift`, `kotlin`, `scala`, `php`, `dart`, `elixir`,
`erlang`, `racket`, `mysql`, and more. Set a default with
`lcx config set lang <slug>` or override per command with `--lang`.

## Roadmap

- leetcode.cn endpoint support
- Tag/difficulty/status filters inside the TUI browser
- Syntax highlighting in the solve-view code preview
- Submission history browsing

## Contributing

Contributions are welcome! Before opening a pull request, make sure the following
pass:

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

Guidelines:

- Branch off `main` (`feat/…`, `fix/…`, `chore/…`).
- Use [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`,
  `fix:`, `chore:`, …); the type drives the version bump.
- Follow [Semantic Versioning](https://semver.org/); the version lives in
  `Cargo.toml`, and notable changes go in [`CHANGELOG.md`](CHANGELOG.md).
- Keep the client layer (`src/client`) free of presentation logic and reuse the
  shared helpers.
- Never commit credentials (`config.toml`, session/csrf cookies).

> LeetCode's endpoints are unofficial and may change; if requests start failing,
> refresh your cookies with `lcx login`.

## License

MIT
