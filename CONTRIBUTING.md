# Contributing to bettercut

Thanks for looking. bettercut is in public beta, so the most valuable thing is
a good bug report; code is welcome too.

## Reporting a bug

In the app, press **Ctrl+K** and choose **Report a Bug on GitHub**. It opens a
new issue with the version, your system and the shape of the project (lanes,
clip counts, resolution) filled in — never file names or anything from your
footage. Add what you did, what you expected, and what happened.

If bettercut crashed, it shows the saved report the next time it starts; the
**Report on GitHub** button copies it and opens an issue. The report includes
recent log lines, so read it over before posting it publicly.

A small file that reproduces a problem is worth more than any description. If
you can share one, attach it or say where it came from (phone model, camera,
app that exported it).

## Changing the code

Setup and the build are in the [README](README.md#building-from-source). Before
a pull request:

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The GPU tests skip themselves on machines without a usable adapter.

A few rules the code relies on — [docs/status.md](docs/status.md) explains each:

- **Time is integer ticks, never floats.** 960,000 a second, which divides
  exactly by every NTSC rate.
- **The interface never changes the project directly.** Every edit is a
  command, which is what gives undo, automation and templates one mechanism.
- **Preview and export are one render path.** Nothing may draw a picture one
  way on screen and another way in the exported file.
- **No GPL components.** FFmpeg is linked as an LGPL build; the setup script
  refuses anything else.

`development_guide.md` is the design document the code cites as `§` numbers.
It is a fixed reference: change the code and the docs around it, not the guide.

By contributing you agree your work is licensed under the project's terms,
MIT OR Apache-2.0.
