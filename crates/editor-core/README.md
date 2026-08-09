# bettercut-editor-core

## Purpose

Owns the authoritative project state and the only route by which it changes:
commands (§54, §55). Provides undo/redo (§11), grouped commands (§79), and the
event stream the UI subscribes to (§56).

## The rule this crate exists to enforce

```text
UI reads   → borrowed snapshot or query, per frame
UI writes  → commands only, never direct mutation
```

`Editor` exposes `project()` returning `&Project` and **no** `project_mut()`.
Rust would happily allow direct mutation; the API does not, because direct
mutation bypasses undo and breaks automation (§32) and templates (§77), both of
which are defined as command generators.

## Public interfaces

| Item | Purpose |
|---|---|
| `Editor` | Owns `Project`, history, dirty flag, playhead. The §55 API surface. |
| `Command` | Serializable request enum — what the UI sends, what §38.2 journals. |
| `EditorCommand` | Trait for an executed, reversible operation (§10). |
| `CommandGroup` | Many commands, one undo step (§79). |
| `Event`, `EventReceiver` | Bounded-channel notifications (§56). |

## Threading assumptions

`Editor` is **not** thread-safe by design and lives on the UI thread. Heavy work
does not happen here — it is dispatched as jobs and reports back via `Event`.
`Event` is `Send`, so background threads can emit into the bounded channel.

Events are delivered over a bounded channel that **drops rather than blocks**
when full. A slow UI frame must never stall a decode or export thread (§56).

## Data ownership

`Editor` owns the one authoritative `Project`. Commands own their undo payload:
`RemoveClip` holds the removed clip so undo can put it back, rather than
re-deriving it.

## Error behavior

`EditorError` for every rejection. A command that fails leaves the project
unchanged and is **not** pushed onto the undo stack — a failed edit must not
become an undoable step.

## Performance assumptions

* Undo is bounded at 300 entries (§11 says 100–500). Oldest entries drop.
* Commands do not snapshot the project (§11). They store only what undo needs.
* No allocation per UI frame in the read path: `project()` is a borrow.
