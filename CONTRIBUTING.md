# Contributing

Thanks for wanting to help. Issues and pull requests are welcome.

This repository receives one commit per release, from a private working repository, so
its history is a list of releases and not of changes. That shapes how a pull request
travels; see "What happens to your pull request" below.

## Building and running

    ./build.sh          # release build
    ./build.sh run      # build, then start hyprscope

hyprscope needs a running Hyprland session to show anything, but the tests do not:
they use fake sockets and the files under `tests/fixtures/`.

## Checks

These three commands are run automatically on every pull request, and again on each
release commit that lands on `main`: the build and the tests run on the stable Rust
toolchain and on 1.88, the oldest the crate supports, and clippy runs on stable only.
Run them before you open a pull request:

    cargo build --locked
    cargo test --locked
    cargo clippy --all-targets --locked -- -D warnings

## Pull requests

- Make one change per pull request.
- Add tests for behaviour that changes.
- When a user would notice the change, add a line under "Unreleased" in CHANGELOG.md.
- If the screen changes, regenerate the picture in the README with
  `python3 docs/make_screenshot.py`.

A change in behaviour is easier to accept when it was discussed in an issue first.
hyprscope only reads from Hyprland; a change that makes it act on the compositor needs
to be discussed before any work is done on it.

## What happens to your pull request

If a pull request is accepted, it is applied to the private working repository by hand
and the pull request is then closed, not merged. GitHub shows it as closed for that
reason, whether or not the change was taken. The change reaches this repository with
the next release, credited to you by name in the changelog and in the release commit's
message.

## Licence

By contributing you agree that your work is released under the project's MIT licence.
