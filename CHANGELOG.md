# Changelog

## Unreleased

### New

- **A zoom slider** with - and + in the corner above the track heads, as in
  CapCut. Ctrl+wheel and the Timeline menu still zoom too.
- **An Import card** fills the empty media panel, as in CapCut: click it to
  choose files, or drop them on it.

## 0.6.5 — Transitions between whole clips; a hang fixed; CapCut-style timeline

Still a beta: expect bugs, and please
[report them](https://github.com/mentiacademyedu/bettercut/issues).

### New

- **Split, Delete, Trim Start and Trim End buttons** above the timeline,
  where CapCut keeps them, so the everyday cuts can be found without the
  keys (each button names its key). Left out when the window is too narrow.
- **Transitions between clips placed whole.** Two clips dropped in from
  start to end have no footage past the cut, so a crossfade, slide or wipe
  between them used to be greyed out. Now the clips overlap to make room,
  as in CapCut: each gives up half the transition and the edit gets that
  much shorter. The menu says by how much before you choose. "Transition
  on Every Cut" does the same.

### Fixed

- **A hang when two jobs opened video at the same moment** — for example an
  assistant detecting scenes while an export started. Opening a decoder,
  an encoder or a graphics device now happens one at a time; once open
  they still run side by side.
- **New titles are the same size in a 4K project as in HD.** Title sizes
  are in the frame's own pixels, so in 4K a new title, caption, lower
  third, sticker or shape came out half as big. They are now made for the
  frame they go in (by its shorter side, so a vertical video matches too).
- **Waveforms you can read.** Sound at an ordinary level drew as a thin
  line; waveforms are now drawn in decibels and fill the clip as in
  CapCut, follow the clip's volume (a clip turned down, or muted, shows
  it), and sit darker than the clip so its name stays readable.
- **Timeline thumbnails keep the picture's shape.** Zoomed out, each
  thumbnail was squeezed to a sliver holding a whole frame; now every tile
  is the video's own shape (narrow for a phone video) and shows the frame
  under it, as in CapCut.
- At start-up, the welcome window no longer sits on top of the "Recover
  unsaved work?" prompt: windows waiting at start come one at a time.
- The empty timeline's hint is a drop box on the main lane, readable at any
  lane height, instead of two lines of text cut through by the lanes.

## 0.6.4 — An assistant shapes the sound, the lanes and the whole video

Still a beta: expect bugs, and please
[report them](https://github.com/mentiacademyedu/bettercut/issues).

### New

- **`shape_sound`**: an assistant sets a clip's EQ (Voice, Phone, Radio,
  Megaphone…), voice (chipmunk, deep or any pitch), echo or reverb and
  robot voice in one undo step.
- **`close_gaps`**, **`loop_clip`**, **`boomerang`**,
  **`transition_every_cut`**, **`fit_music`**, **`group_clips`**,
  **`hold_last_frame`** and **`split_into`** — the timeline menu's
  structural edits, for an assistant.
- **`whole_video_look`**: cinematic bars, a progress bar, vignette, grain
  and the background colour over the whole video, in one undo step.
- **`set_lane`**: rename a lane, lock it, switch it off, solo it or set a
  sound lane's volume. `describe_project` now shows each lane's state.
- **`organise_media`** renames an imported file and files it in a bin;
  **`remove_unused_media`** clears out what no clip uses.
- Two new ready-made requests: **`make_it_cinematic`** (bars, one look,
  soft cuts, music that ends with the pictures) and **`clean_up_sound`**.
- **`remove_range`**: take out everything between two times on every lane,
  closing the gap or leaving it, in one undo step.

### Fixed

- An assistant deleting a shot left its sound behind, and a ripple delete
  left that sound out of sync with everything after it. The shot's sound
  now goes with it, as Delete does in the app.

## 0.6.3 — One undo for an assistant's whole change; chapters and subtitles

Still a beta: expect bugs, and please
[report them](https://github.com/mentiacademyedu/bettercut/issues).

### New

- **`batch`**: an assistant makes several edits as one undo step, so its
  whole change comes back with one Ctrl+Z.
- **`chapters`** gives the YouTube chapter list from the markers, and
  **`export_captions`** writes the captions to an `.srt` or `.vtt` file.

## 0.6.2 — Many cuts at once are fast

Still a beta: expect bugs, and please
[report them](https://github.com/mentiacademyedu/bettercut/issues).

### Fixed

- **Many cuts at once are fast.** Splitting a clip into a thousand pieces
  (split every few seconds, at every marker or scene) took forty-five
  seconds: the crash-recovery journal went to the disk after every one.
  It writes each step once now — a quarter of a second.

## 0.6.1 — Fits small screens; light theme fixes

Still a beta: expect bugs, and please
[report them](https://github.com/mentiacademyedu/bettercut/issues).

### Fixed

- **Windows open below the toolbar.** Storyboard, Notes, Scopes, Exports,
  Captions, Markers, History, Trim, Sound in Detail, What Changed and Find
  the Good Bits all opened in the top-left corner, over Play, New and Open.
- **The light theme's chosen buttons show their words** ("All", "Video",
  "Fit" read as empty blue boxes), and names on colour clips read on dark
  colours.
- **The Colours tab fits the inspector.** The sliders under the colour
  wheels made it twice as wide, so the preview covered the inspector's left
  edge — its tabs and filter names cut off.
- **A long file name no longer widens the media panel.** Camera names have
  no spaces to wrap at, so one pushed the panel wide; it is cut short with
  "…" now, and shown whole on hover.
- **The Export window fits a small screen.** In a short window it ran off
  the top and bottom, Export and Cancel out of reach; its settings scroll
  now, and the buttons stay.
- **The toolbar takes a second row in a narrow window** instead of drawing
  the timecode, Actions and Windows over its buttons.
- **A small window keeps a usable preview**: the timeline starts at two
  fifths of the window (never more than before), and the status bar's
  shortcut reminder steps aside rather than crowd the clip count.

### New

- **"Show Effects", "Show Colours"…** in Ctrl+K jump to an inspector tab.
- **`contact_sheet`**: an assistant sees the whole edit in one picture —
  frames spread through it, in a grid — to check a cut in one look.

## 0.6.0 — Hear about new versions; assistants export GIFs

Still a beta: expect bugs, and please
[report them](https://github.com/mentiacademyedu/bettercut/issues).

### New

- **Assistants export GIFs, sound and masters**: the file's ending picks
  the kind — `.gif` (a small looping animation), `.wav`, `.mov` (ProRes),
  `.webm` (see-through background) — besides `.mp4`.
- **New versions are announced**: once a day the app asks GitHub whether a
  newer bettercut is out and, if so, says so in the status bar with a link.
  Nothing is sent but the request; Settings turns it off.
- **"assistant connected"** in the status bar while an AI assistant is
  working in the window; a click shows how it is connected.
- **Assistants see what is on a clip**: `describe_project` lists each
  shot's effects, and whether it is keyed or cropped.

### For developers

- Tests no longer leave unsaved "recovered work" in the machine's temp
  folder for the app to offer at its next launch: the repository's cargo
  config points `BETTERCUT_RECOVERY_ROOT` at `target/recovery`.

## 0.5.2 — Crash recovery: undo, and projects side by side

Still a beta: expect bugs, and please
[report them](https://github.com/mentiacademyedu/bettercut/issues).

### Fixed

- **Undo and redo are kept for crash recovery.** They were never written
  down: after a crash, edits you had undone came back, and ones you had
  redone were lost. Every assistant session in the tests now checks that
  recovery brings the project back exactly.
- **Projects saved in the same folder keep separate crash data.** They all
  used one `recovery` folder, so one could overwrite another's — and opening
  one could offer the other's work. Each now has `recovery/<file name>`;
  data in the old shared folder is still offered, but only to its own
  project.

## 0.5.1 — Title edits survive a crash; more for assistants

Still a beta: expect bugs, and please
[report them](https://github.com/mentiacademyedu/bettercut/issues).

### Fixed

- **Changing a title's words is saved for crash recovery again.** The
  autosave journal could not write it (nor a title's rotation, opacity or
  motion blur), so the status bar said "autosave failed once" and a crash
  lost those edits.
- **Titles fit a narrower shape.** A copy of the edit as 9:16 (or another
  narrower shape, or an export in one) wraps each title inside the new frame
  instead of letting it run off both sides.

### New

- **Ready-made requests**: the MCP server offers prompts — tighten an
  interview, make a Short, cut to the beat, a title card — which Claude Code
  shows as slash commands (`/bettercut:make_short` and so on).
- **`green_screen`** keys out a green or blue backdrop, and **`crop`** cuts
  the edges off a shot.
- **Background exports for assistants**: `export` with `wait` false starts
  the render and answers at once — no client timeout on a long export — and
  `export_status` follows it.

## 0.5.0 — Connect an assistant in one click; 55 tools

Still a beta: expect bugs, and please
[report them](https://github.com/mentiacademyedu/bettercut/issues).

### New

- **Connect an AI Assistant** (Windows menu, or Ctrl+K): the exact setup
  command for this install, ready to copy, for Claude Code and Claude Desktop
  — and whether an assistant is connected to the window right now. **Add to
  Claude Code** and **Add to Claude Desktop** buttons do the setup in one click
  (Claude Desktop's other settings are kept, with a copy saved first).
- **MCP on Linux**: the AppImage serves MCP when run with `--mcp`
  (`bettercut --mcp` works everywhere), since an AppImage runs one program.
- **Keyframes for assistants**: `animate` sets a clip's opacity, position,
  scale, rotation, colour, blur — or a sound's volume and pan — changing over
  time, with easing, replacing what was there in one undo step.
  `describe_project` lists what each clip has animated.
- **Templates for assistants**: `list_templates` shows bettercut's starters
  and your own templates with the shots and words each asks for;
  `apply_template` builds one on the timeline from your media and words.
- **Sound for assistants**: `normalise_volume`, `duck_under_voice` (music
  dips wherever someone speaks over it), `enhance_voice` and `mute_clip`.
- **`remove_silences`**: an assistant cuts the pauses out of someone talking
  and closes the gaps — or, with `preview`, just lists them first.
- **"This clip"**: attached to the window, an assistant can ask what you
  have selected and where the playhead is (`get_selection`), and select clips
  itself to show you which ones it means (`select_clips`); `set_playhead`
  moves the playhead.
- **Layout for assistants**: `add_colour` (a colour or gradient
  background), `freeze_frame`, `picture_in_picture`, and `copy_as_shape` —
  a vertical 9:16 (or square, 4:5, 21:9) copy of the whole edit, every shot
  reframed — with `switch_sequence` to move between them.
- **Graphics for assistants**: `add_lower_third` (a name and role with a
  coloured bar), `add_shape`, `add_sticker` and `add_timer`; and
  `set_effect` — blur, sharpen, vignette, glow, old film, glitch, RGB split,
  pixelate, zoom blur, light leak, lens flare, beat pulse or smooth skin.
- **`split_at_scenes`** cuts a long recording into its shots, and
  **`mark_beats`** puts a marker on every beat of a song, with its tempo.

## 0.4.0 — An assistant in the open window

Still a beta: expect bugs, and please
[report them](https://github.com/mentiacademyedu/bettercut/issues).

### New

- **An assistant can edit the project open in the window, live**:
  `attach_to_app` connects `bettercut-mcp` to the running app, and every tool
  after it acts there — on screen as it happens, on the app's own undo
  history. It listens on this computer only, guarded by a random key the app
  writes to its settings folder; exports render a copy, so the window stays
  usable while they run.
- **More for assistants**: `set_transform` (place, size and turn a clip or
  title), `style_title` (words, size, colour, bold, italic, outline, a box
  behind), `adjust_colour` (brightness, contrast, saturation, warmth) and
  `add_marker`, `set_movement` (a slow zoom or pan over a photo) and
  `animate_title` (how a title arrives, leaves and moves in between), and
  `add_captions` (subtitles written straight from a list of timed lines).
  Each is one undo step; `describe_project` now reports where clips sit,
  title sizes and colours, and the markers.
- **The status bar says when an attached assistant changes something.**

## 0.3.0 — Edit with an AI assistant, and Intel Macs

Still a beta: expect bugs, and please
[report them](https://github.com/mentiacademyedu/bettercut/issues).

### New

- **MCP server (`bettercut-mcp`)**: AI assistants such as Claude can create
  and open projects, import media, place, move, trim, split and delete clips,
  add titles and captions, set volume, opacity, speed, fades, transitions and
  filters, undo and export, through the editor's own undoable commands — and
  see any frame of the edit as an image (`preview_frame`) to check their work.
  Installed beside the app on Windows and Mac; setup in the README.
- **The app notices when its open project is saved by another program** — an
  assistant, a sync folder — and offers to load the new version (or keep the
  one on screen), warning first if that would discard unsaved work.
- **Intel Macs**: a `macos-intel` disk image beside the Apple Silicon one.
- **Linux (beta)**: an AppImage, `bettercut-0.3.0-x86_64.AppImage` — download,
  `chmod +x`, run. Built and tested on Ubuntu; it needs the system's own
  graphics driver (Vulkan) and sound.

### Known limitations

- **The Mac app is not signed by Apple.** On first open macOS refuses it: open
  System Settings → Privacy & Security and choose **Open Anyway**.
- **The Windows installer is not code-signed**: SmartScreen asks you to
  confirm (*More info → Run anyway*).
- **The MCP server edits project files**, not the open window: the app offers
  to load what an assistant saved.
- **No automatic captions from speech, background removal or auto-reframe**
  yet; they need on-device models.
- **No auto-update**: install a new version over the old one.

## 0.2.0 — Mac beta, and iPhone media

Still a beta: expect bugs, and please
[report them](https://github.com/mentiacademyedu/bettercut/issues).

### New

- **macOS (Apple Silicon), first beta.** A `.dmg` beside the Windows
  installer. Settings and caches go where a Mac keeps them (`~/Library`),
  text-to-speech uses the Mac's own voices, export uses Apple's VideoToolbox,
  and shortcuts show as ⌘ and Option. Built and tested on GitHub's Mac
  machines; **nobody on the team owns a Mac yet**, so Mac testers are very
  welcome.
- **iPhone photos (HEIC/HEIF)** import: the tiles a phone stores a photo in are
  stitched, cropped to the shown size and turned upright, up to 48 megapixels.
- **Live Photos**: a photo's short video is found beside it and offered in the
  card's More menu.
- **iPhone video**: recordings with Spatial Audio no longer come in silent (the
  playable AAC track is used), and jumps into variable-frame-rate video land on
  the right frame.
- **Phone and camera files**: portrait video and EXIF-rotated photos import
  upright; camera `.ts`/`.mts` files whose clock starts late play from their
  start.
- The export bar says how much time is left; songs with cover art import as
  sound rather than as a still.

### Fixed

- Exports made without a hardware encoder were one frame short, so a reversed
  clip started a frame early.

### Known limitations

- **The Mac app is not signed by Apple.** On first open macOS refuses it: open
  System Settings → Privacy & Security and choose **Open Anyway**. Apple
  Silicon only; Intel Macs are not built yet.
- **The Windows installer is not code-signed**: SmartScreen asks you to
  confirm (*More info → Run anyway*).
- **No automatic captions from speech, background removal or auto-reframe**
  yet; they need on-device models.
- **No auto-update**: install a new version over the old one.


## 0.1.0 — first public beta

The first release anyone can download. It is a beta, and it will have bugs:
it has been tested on two Windows PCs, a desktop and a laptop, mostly with
generated test footage.
Please [report what breaks](https://github.com/mentiacademyedu/bettercut/issues)
(Ctrl+K → *Report a Bug on GitHub* fills in the details for you).

### In this release

- A full timeline editor: lanes, snapping, magnetic main track, ripple, roll
  and slip edits, groups, compound clips, markers, several sequences, undo
  history and crash recovery.
- Picture: keyframes and motion paths, masks, green screen, blend modes,
  colour wheels, curves, LUTs, filters and effects, stabilisation, tracking,
  speed ramps, picture-in-picture, split screens, and backdrops for vertical
  video.
- Text: styled titles with animations, captions (SRT/VTT) and templates.
- Sound: EQ presets, voice clean-up, Enhance Voice, leveller, de-esser, voice
  effects, ducking, loudness, beat detection, silence removal, voice-over and
  text-to-speech.
- Export: H.264, H.265 and AV1 on supporting GPUs, ProRes 422 HQ, GIF,
  audio-only and stills, with an export queue.
- Real-world footage: portrait phone video and rotated photos import upright,
  HDR is tone-mapped, and camera `.ts`/`.mts` files play from their start.
- A Windows installer (per-user, no administrator rights) and a portable zip.

### Known limitations

- **Windows only.** (A Mac beta followed in 0.2.0.)
- **The installer is not code-signed**, so SmartScreen asks you to confirm:
  *More info → Run anyway*.
- **Tested on two machines** (a desktop with an NVIDIA GPU, and a laptop).
  Performance on low-end laptops with integrated graphics, the target, has
  not been measured yet.
- **HEIC photos** (the iPhone default) do not import yet. (They do from
  0.2.0.)
- **No automatic captions from speech, background removal or auto-reframe**
  yet; they need on-device models.
- **No auto-update**: install new versions over the old one.
- Projects are saved in a format that may still change before 1.0; older
  projects will be opened and upgraded, but keep backups of anything important.

### Coming next

- **MCP integration**: an MCP server so AI assistants can edit through the
  same undoable commands as the interface.
- Automatic captions, background removal and auto-reframe as optional
  downloads.
- HEIC, code signing, auto-update, macOS and Linux.
