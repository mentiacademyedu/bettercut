# Changelog

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

- **Windows only.** macOS and Linux builds are planned.
- **The installer is not code-signed**, so SmartScreen asks you to confirm:
  *More info → Run anyway*.
- **Tested on two machines** (a desktop with an NVIDIA GPU, and a laptop).
  Performance on low-end laptops with integrated graphics, the target, has
  not been measured yet.
- **HEIC photos** (the iPhone default) do not import yet.
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
