# bettercut-project-format

## Purpose

The `Project` aggregate and its on-disk representation: human-readable versioned
JSON in a `.vproj` file (§37), written atomically (§38.1).

## Public interfaces

| Item | Purpose |
|---|---|
| `Project` | Media library + sequences + settings (§8). |
| `ProjectSettings` | Performance mode, cache limits, autosave interval (§43). |
| `save(project, path)` | Atomic write: tmp → fsync → rename (§38.1). |
| `load(path)` | Read, version-check, migrate (§37). |
| `SCHEMA_VERSION` | Bumped whenever the on-disk shape changes. |

## Threading assumptions

`save`/`load` block on I/O and must not be called from the UI thread (§2). They
take `&Project` and return owned values — no shared state, no locks.

## Data ownership

`Project` owns its media list and sequences. It stores **references** to media —
paths and metadata — never media content (§2).

## Error behavior

`ProjectError` distinguishes I/O failure, malformed JSON, and an unsupported
schema version. A newer-than-supported file is refused with a clear message
rather than partially parsed.

**The atomic write is the important part.** §38.1: never truncate the project
file in place, because a crash mid-write destroys the project. The sequence is:

```text
1. write project.vproj.tmp
2. fsync the file
3. fsync the directory      (so the rename itself is durable)
4. rename .tmp -> .vproj    (atomic on all target platforms)
```

If any step fails the original file is untouched.

## Performance assumptions

JSON is acceptable at MVP scale (§37). The threshold for revisiting it is stated
there: if §52's 10,000-clip benchmark shows serialization exceeding ~100 ms, the
migration path is a binary format behind this same interface.

`save` is O(project size). §38.2's command journal exists so that autosave does
*not* pay this cost on every edit; that lands in Milestone 5.
