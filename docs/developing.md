# Developing bettercut

The switches, checks and habits that make working on bettercut quicker. For
setting up the build, see the [README](../README.md#building-from-source); for
what the code is built around, [status.md](status.md).

## Running the tests

```powershell
cargo test --workspace -j 4
```

`-j 4` (or less) on a machine with 16 GB or so: every test file links its own
binary, and the default parallelism can run the linker out of memory. One crate
at a time (`-p bettercut-editor-core`) is gentler still.

The debug build cache grows large — hundreds of gigabytes over weeks of builds.
`cargo clean --profile dev` takes it back; the next build is a few minutes
slower.

Some checks guard whole categories rather than one feature. When one fails, it
is telling you something about the change, not about the test:

- **Every assistant session** (`apps/mcp/tests/session.rs`) ends by checking
  that every edit reached the crash-recovery journal, that recovery rebuilds
  the project exactly, that saving and loading changes nothing, and that
  undoing and redoing everything comes back to the same project.
- **Every clip and title property** (`crates/editor-core/tests/every_clip_property.rs`,
  `every_text_property.rs`) is set, undone, redone and recovered after a
  simulated crash, exactly.
- **Every window** says where it first appears (`crates/ui/tests/windows_placed.rs`),
  and every inspector tab and the media panel fit at their narrowest
  (`inspector_width.rs`).
- **Every string and symbol** in the interface is checked: no lost line breaks
  (`interface_text.rs`), no character the font cannot draw (`glyphs.rs`).
- **Big edits stay quick**: a thousand clips cut, undone, deleted and drawn
  (`big_edit_speed.rs`, `big_edit_frame_time.rs`).

GPU tests skip themselves where there is no usable adapter.

## Switches for looking at the app

The app reads a few environment variables, for seeing it as a person would —
always its **own** window, never the screen:

| Variable | What it does |
|---|---|
| `BETTERCUT_SAMPLE=1` | Opens the sample edit instead of an empty project. |
| `BETTERCUT_SCREENSHOT=path.png` | Saves a picture of the window once it has settled, then quits. |
| `BETTERCUT_PALETTE="Export; Show Colours"` | Runs these command-palette actions at start — to photograph a window or an inspector tab. |
| `BETTERCUT_WINDOW=1280x720` | Starts the window at this size (the smallest is 900x560). |
| `BETTERCUT_RECORD=folder` | Saves the window five times a second as numbered PNGs, then quits after `BETTERCUT_RECORD_SECONDS` (default 30). How the README's assistant demo was made. |
| `BETTERCUT_WRITE_ICONS=folder` | Writes the app icon at every size and quits (the Mac packaging uses it). |

For example, the Colours tab in a small window:

```powershell
$env:BETTERCUT_SAMPLE=1; $env:BETTERCUT_WINDOW="900x560"
$env:BETTERCUT_PALETTE="Show Colours"; $env:BETTERCUT_SCREENSHOT="colours.png"
cargo run -p bettercut-desktop
```

Point `APPDATA`, `LOCALAPPDATA` and `TEMP` at a scratch folder for these runs,
so they neither read nor change your own settings and recent projects.

## Other switches

| Variable | What it does |
|---|---|
| `BETTERCUT_FORCE_ENCODER=h264_nvenc` | Exports only with this encoder, to test one encoder's output. |
| `BETTERCUT_UPDATE_GOLDEN=1` | Rewrites the renderer's golden frames — read `crates/renderer/tests/golden_frames.rs` first. |
| `BETTERCUT_RECOVERY_ROOT=folder` | Where unsaved projects keep crash-recovery data. The repository's cargo config sets it to `target/recovery`, so tests never leave "recovered work" for the real app to offer. |
| `BETTERCUT_FFMPEG_BIN=folder` | Where the FFmpeg DLLs are copied from at build time (set by the cargo config). |

## The MCP server

`bettercut-mcp` (and `bettercut --mcp`) speak MCP on standard input and
output, so a session can be driven by hand:

```bash
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | target/debug/bettercut-mcp
```

With the app running, `attach_to_app` reaches its window through
`live.json` in the settings folder (`%APPDATA%\bettercut` on Windows).
