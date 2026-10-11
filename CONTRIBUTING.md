# Contributing

Thanks for wanting to help. Issues and pull requests are welcome.

`main` is the public branch. Each release is a tagged commit on it, and the maintainer's
own work arrives there with a release.

## Building and running

    cargo build --release   # release build
    cargo run --release     # build, then start hyprscope

hyprscope needs a running Hyprland session to show anything, but the tests do not:
they use fake sockets and the files under `tests/fixtures/`.

## Checks

These three commands are run automatically on every pull request, and again on each
push to `main`: the build and the tests run on the stable Rust
toolchain and on 1.88, the oldest the crate supports, and clippy runs on stable only.
Run them before you open a pull request:

    cargo build --locked
    cargo test --locked
    cargo clippy --all-targets --locked -- -D warnings

## Pull requests

- Make one change per pull request, against `main`.
- Add tests for behaviour that changes.
- When a user would notice the change, add a line under "Unreleased" in CHANGELOG.md.
- If the screen changes, regenerate the picture in the README with
  `python3 docs/make_screenshot.py`.

A change in behaviour is easier to accept when it was discussed in an issue first.
hyprscope only reads from Hyprland; a change that makes it act on the compositor needs
to be discussed before any work is done on it.

## What happens to your pull request

A pull request made against `main` is reviewed and, if accepted, merged in the usual way,
so your commits stay in the history under your name. The next release includes it,
and the changelog credits you by name.

## Licence

By contributing you agree that your work is released under the project's MIT licence.
