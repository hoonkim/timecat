# Timecat

A small Rust digital clock for kitty and other modern terminals.

It renders a large 7-segment style clock with white digits on a black
background.

## Run

```sh
cargo run
```

The clock opens in the terminal alternate screen and updates once per second.
Press `q`, `Esc`, or `Ctrl-C` to quit.

## Install With Homebrew

```sh
brew install hoonkim/tap/timecat
```

## Build

```sh
cargo build --release
```

## Release

Publishing a GitHub release automatically updates `hoonkim/homebrew-tap`.
The workflow needs a repository secret named `HOMEBREW_TAP_TOKEN` with write
access to the tap repository.
