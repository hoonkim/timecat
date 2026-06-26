# Timecat

A small Rust digital clock for kitty and other modern terminals.

It renders a large 7-segment style clock with white digits on a black
background. When Google Workspace CLI is configured, it also shows your current
and next calendar events and flashes before meetings start.

## Run

```sh
timecat
```

The clock opens in the terminal alternate screen and updates once per second.
Press `q`, `Esc`, or `Ctrl-C` to quit. When a calendar alert is active,
`Esc` or any mouse click dismisses the alert instead.

## Install With Homebrew

```sh
brew install hoonkim/tap/timecat
```

## Google Calendar

Timecat can show the current and next calendar event at the bottom of the
screen. It flashes the clock 10 minutes before an event starts and again when
the event starts. If the event has a room resource, Timecat displays it after
the event title.

The default integration uses `gws`, the Google Workspace CLI:
https://github.com/googleworkspace/cli

```sh
gws auth login
timecat
```

If `gws` is already logged in and can read your calendar, `timecat` can read it
too. By default Timecat reads the signed-in user's primary calendar through:

```sh
gws calendar events list --params '{"calendarId":"primary","singleEvents":true,"orderBy":"startTime"}'
```

Set `TIMECAT_GWS_CALENDAR_ID` to read another calendar ID.

`gcalcli` is still supported as a fallback when `gws` is not installed:

```sh
pipx install gcalcli
gcalcli agenda
timecat
```

You can also provide your own command:

```sh
TIMECAT_CALENDAR_CMD='my-calendar-script' timecat
```

The command should print tab-separated events in one of these formats:

```text
2026-06-26 09:00	2026-06-26 09:30	Standup
2026-06-26	09:00	2026-06-26	09:30	Standup
```

## Build

```sh
cargo build --release
```

## Release

Publishing a GitHub release automatically updates `hoonkim/homebrew-tap`.
The workflow needs a repository secret named `HOMEBREW_TAP_TOKEN` with write
access to the tap repository.
