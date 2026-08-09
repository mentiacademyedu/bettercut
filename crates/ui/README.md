# bettercut-ui

## Purpose

egui widgets and the canvas-drawn timeline (§53, §58). This crate is the top of
§86's stack: it depends on `editor-core`, and nothing depends on it.

## Public interfaces

| Item | Purpose |
|---|---|
| `UiState` | View state: zoom, scroll, selection, status line. Not project data. |
| `draw(ui, editor, state)` | Renders the whole §58 layout for one frame. |
| `Shortcut` | §57's key bindings, resolved to intents. |
| `timeline::draw` | The §53 canvas. |

## Threading assumptions

Runs entirely on the UI thread. Every method takes `&mut Editor` and returns
before the frame ends. Nothing here blocks on I/O except the native file dialogs,
which are modal by nature.

## Data ownership

`UiState` owns **view** state only — zoom, scroll offset, selection, the status
line. None of it is saved in the project.

The UI never owns or mutates `Project`. It reads through `editor.project()` and
writes only by dispatching commands (§54). There is no `project_mut()` to call.

## Error behavior

A command that fails is shown in the status line and otherwise ignored — an
invalid drag or an overlapping drop must not interrupt editing (§50).

## Performance assumptions

* **The timeline is a canvas, not a widget tree** (§53). Everything is drawn with
  one `Painter` inside one allocated rect. 10,000 clips do not become 10,000
  widgets.
* **Virtualized by viewport.** Only clips intersecting the visible time range are
  drawn, via `Track::clips_in_range`.
* **A timeline repaint never triggers a decode** (§53). Waveforms and thumbnails
  will read from cache only; until those caches exist, they are simply not drawn.
* **Zoom and scroll are integers.** `ticks_per_pixel` is an `i64` from a fixed
  ladder, so pixel↔tick conversion contains no floating-point timeline
  arithmetic (§74).
* Repaint is reactive: the app requests a repaint only when something changed
  (§81's ~0 fps idle target).
