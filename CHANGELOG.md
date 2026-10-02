# Changelog

## Unreleased

- **MCP server (`bettercut-mcp`)**: AI assistants such as Claude can create
  and open projects, import media, place clips, add titles, split, delete,
  undo and export, through the editor's own undoable commands. Installed
  beside the app on Windows and Mac; setup in the README.

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
