# Contributing

Thanks for wanting to help. Issues and pull requests are welcome.

## Building and running

    ./build.sh          # release build
    ./build.sh run      # build, then start hyprscope

hyprscope needs a running Hyprland session to show anything, but the tests do not:
they use fake sockets and the files under `tests/fixtures/`.

## Checks

Pull requests are checked automatically with these three commands, on the stable Rust
toolchain and on 1.88, the oldest the crate supports. Run them before you open one:

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

## Licence

By contributing you agree that your work is released under the project's MIT licence.
