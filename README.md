<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/wordmark-dark.svg">
    <img src="assets/wordmark-light.svg" alt="allock" height="140">
  </picture>
</h1>
<p align="center">A lock screen and greetd greeter for Wayland, drawn as an island.</p>
<p align="center">
    <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-GPL--3.0--or--later-blue"></a>
</p>

> [!NOTE]
> This project has been developed with the assistance of AI.

<p align="center">
  <img src="assets/screenshots/demo.webp" alt="The lock in action: the island drops in, refuses a wrong password, takes the right one and goes back up" width="100%">
</p>
<p align="center">
  <img src="assets/screenshots/greeter.webp" alt="The greeter, with its session picker" width="49%">
</p>

## ✨ Features

- Lock screen through `ext-session-lock-v1`, one surface per monitor, hotplug included
- greetd greeter with a user picker and a session picker
- One island: it grows out of a dot at the top straight into the login card,
  with the fingerprint reader listening, folds up into a pill at the top when
  nobody is there, and back into the dot when you are in
- Type to unlock: the key that wakes the pill is the first character of the password
- Password through PAM, fingerprint through fprintd (lock screen only)
- Works on its own, with any bar or none; a bar that draws an island of its
  own can hand it over through an optional D-Bus handoff, off by default
- Blurred wallpaper, or a snapshot of the session that blurs as the lock
  starts and sharpens as it ends, a rolling clock, battery ring, suspend, reboot and shut down
- Multi-monitor: choose which monitors show the controls
- Remembers the last user and session in the greeter
- Preview mode to try a theme without locking

## 🛠️ Install

```sh
scripts/install.sh             # lock screen (the default)
scripts/install.sh --greeter   # greeter
scripts/install.sh --all       # both
```

The script builds allock and installs it to `/usr/local/bin`, then what each
mode needs: the PAM policy `pam/allock` to `/etc/pam.d/allock` for the lock
screen and the greeter's config under `/etc/greetd`.

See [docs/SETUP.md](docs/SETUP.md) to use it as your lock screen and as your
login screen.

## ⚙️ Configuration

allock works without a config file. To change it, copy
[`config.example.toml`](config.example.toml) to `~/.config/allock/config.toml`
(`$XDG_CONFIG_HOME/allock/config.toml`, or the file given with
`--config-path`): every key is optional and shown there with its default. A
file allock cannot read is reported in its log, and the lock comes up with the
defaults.

The layout is fixed. The config chooses colours, wallpaper, clock format, where
the island rests, which indicators appear and which monitors show the
controls.

### Monitors and log

```toml
log_level = "warn"                # error | warn | info | debug | trace
main_output = "All"               # or { Targets = ["DP-2", "eDP-1"] }
```

Every monitor is covered while locked. `main_output` picks the ones that show
the clock and the card, by connector name or part of the EDID; the others keep
the island at rest. A target that is not connected puts the controls on the
first monitor.

### Background

```toml
[background]
source = "Wallpaper"              # or "Snapshot"
path = "~/Pictures/wallpaper.jpg" # unset: the island's colour, flat
fit = "Cover"                     # Cover | Contain | Stretch
blur = 26.0                       # 0 for a sharp picture
dim = 0.45                        # 0.0 to 1.0
```

`"Snapshot"` starts the lock on a picture of the session, see the island and
the handoff below; `path` stands in where the compositor cannot capture.

### Clock, indicators and authentication

```toml
[clock]
time_format = "%H:%M"             # strftime; a format with %S ticks every second
date_format = "%A %e %B"          # "" hides the date

[indicators]
battery = true

[auth]
pam_service = "allock"            # /etc/pam.d/<name>
fingerprint = true
fingerprint_retries = 5
idle_timeout = 60                 # seconds before the card folds up; 0 keeps it
```

### Island

```toml
[island.seed]
size = 30.0                       # the dot's diameter, logical pixels
top = 6.0                         # from the top edge
```

`[island.seed]` is the dot the island grows out of and goes back into.

### Theme and appearance

`[appearance]` sets the colours and the look of everything allock draws.
Unset, it is Tokyo Night.

The colours come from one of three places. A **theme**, with its flavours and
accent by name (any case; the theme's own when unset):

```toml
[appearance]
theme = "Catppuccin"
dark_flavor = "Macchiato"
light_flavor = "Latte"
accent = "Mauve"
```

| Theme         | Dark flavours            | Light flavour |
| ------------- | ------------------------ | ------------- |
| `Tokyo Night` | Night, Storm, Moon       | Day           |
| `Catppuccin`  | Mocha, Macchiato, Frappé | Latte         |
| `Nord`        | Polar Night              | Snow Storm    |
| `Gruvbox`     | Dark                     | Light         |
| `Dracula`     | Dracula                  |               |
| `Rosé Pine`   | Main, Moon               | Dawn          |
| `Everforest`  | Dark                     | Light         |
| `Kanagawa`    | Wave, Dragon             | Lotus         |

The accents are the flavour's own colour names, like `Blue`, `Green` or
`Magenta` for Tokyo Night; a name the flavour lacks is reported in the log.

Or both flavours worked out from a **seed** colour, as Material You does, or
from the colour that fits a **wallpaper** best (read as the lock starts, so a
large picture makes it start later):

```toml
[appearance]
seed = "#6750a4"                  # or: wallpaper = "~/Pictures/wallpaper.jpg"
variant = "tonal-spot"            # tonal-spot | vibrant | expressive | content |
                                  # fidelity | neutral | monochrome | rainbow | fruit-salad
contrast = "standard"             # standard | medium | high
```

A wallpaper wins over a seed, and either wins over a theme. The greeter runs as
the `greeter` user, so its wallpaper must be somewhere that user can read.

The rest of the look:

```toml
[appearance]
mode = "auto"                     # auto | dark | light; a lock cannot ask the
                                  # desktop, so auto is dark
density = "default"               # compact | default | comfortable
shape = "soft"                    # square | soft | round
material = "solid"                # solid | translucent | glass
motion = "full"                   # full | reduced (fades only) | none

[appearance.fonts]
sans = "Inter"                    # the system's when unset
mono = "JetBrains Mono"
```

Any colour can be set over a flavour, by name: `background` (the island),
`text`, `sub`, `dim`, `accent`, `success`, `warning`, `danger`, and
`on_accent`, `on_success`, `on_warning`, `on_danger` for what reads on each
fill, as `#rrggbb` or `#rrggbbaa`:

```toml
[appearance.dark]
accent = "#7aa2f7"
background = "#16161ee6"

[appearance.light]
accent = "#2e7de9"
```

The look can also come from a file of its own, shared with another program
that wears styletto, such as ashell. It holds the same keys as `[appearance]`,
without the header; what `[appearance]` sets beside the import is written over
it:

```toml
[appearance]
import = "~/.config/styletto/styletto.toml"  # relative to this file, if relative
material = "glass"                           # this one key, for allock alone
```

A file that cannot be read is reported in the log, and `[appearance]` is used
as it is. `scripts/install.sh --greeter` copies the imported file where the
greeter can read it.

### Greeter

The greeter reads `/etc/greetd/allock.toml`, the same keys plus `[greeter]`:

```toml
[greeter]
other_user = true                 # offer "Other…" to type any user name
user = "zibo"                     # preselect this account
default_session = "Niri"          # always start on this session
remember = true                   # start on the last user and session
session_output = "Journal"        # or "Console"

[[greeter.session]]               # your own list; unset, the installed sessions
name = "niri"
command = ["niri-session"]
env = ["XDG_SESSION_TYPE=wayland"]
```

## 🔑 Authentication

The lock screen checks your password through PAM, with the policy in
`/etc/pam.d/allock` that `scripts/install.sh` installs. Without it allock
borrows `system-auth`, `common-auth` or `login`, in that order, and says so in
its log. With none of them it refuses to lock, rather than lock you out.

The policy includes only `system-auth`: the password check (`pam_unix`) and the
limit on wrong attempts (`pam_faillock`). It leaves out the checks for a new
login, like `/etc/nologin` or a shell missing from `/etc/shells`, which would
otherwise keep you out of a session that is already running.

Wrong passwords count: by default three in fifteen minutes lock the account for
ten (`/etc/security/faillock.conf`). A fingerprint still unlocks, because it is
checked by fprintd outside PAM. `faillock --user $USER --reset`, as root, clears
the count. While the account is locked, the island says so as soon as it
appears, and counts down, reading the count the way `faillock` does. It is a warning: the field stays open, and PAM decides.

With `fingerprint = true` (the default) the fingerprint works alongside the
password while the card is up: either one unlocks. At rest the reader is off. Set it to `false` to require the password. The
greeter always asks for the password, and greetd checks it with its own policy,
`/etc/pam.d/greetd`.

## 🔒 Lock screen

```sh
allock             # lock
allock --preview   # draw the lock screen without locking, exits on a correct password
```

Bind `allock` to a key in your compositor, and to your idle daemon's lock and
lid actions.

## 🚪 Greeter

`allock --greeter` runs inside a compositor started by greetd. With
`scripts/install.sh --greeter` the setup is:

```toml
# /etc/greetd/config.toml
[default_session]
command = "systemd-cat -t allock-greeter niri -c /etc/greetd/allock-niri.kdl"
user = "greeter"
```

The greeter's own options go in `/etc/greetd/allock.toml` under `[greeter]`.
[docs/SETUP.md](docs/SETUP.md) has the details.

## 🏝️ The island and the handoff

The island comes in from the top edge of the output and leaves into it. When
the lock takes the screen the compositor cuts from the session to the lock
surface, and no client can animate that cut; when nothing is on screen at the
cut but the background, there is nothing for the eye to see jump.

The background is the only thing the compositor still cuts. With
`[background] source = "Snapshot"` not even that: allock takes a picture of
every output through `wlr-screencopy`, starts the lock on it sharp and blurs
it, and sharpens it again once its own island has gone. It stays in memory and
is never written anywhere; an output that cannot be captured, or a compositor
that takes longer than half a second, gets the wallpaper.

allock needs nothing else on the desktop: whatever bar runs, or none, the lock
comes and goes the same way. The greeter does the same on its side of the
compositor switch: its island goes up into the edge over the wallpaper,
sharpened, which is the picture the session starts on.

### The optional handoff

A bar that draws an island of its own at the top edge can keep it off the
screen at the cut, by implementing `org.allock.Handoff1` on the session bus
([docs/HANDOFF.md](docs/HANDOFF.md)). It is off by default, so a bar that does
not (waybar, or none) is never asked and never waited for:

```toml
[handoff]
enabled = true
timeout_ms = 600                  # how long the bar has to go
```

Enabled, allock calls `PrepareHandoff("lock")` before locking, and once the
answer comes, or `timeout_ms` passes, locks and drops its own island in; after
the unlock, `ResumeAfterHandoff()` lets the bar's island come back. When nobody
owns the name allock locks at once, after one round trip to the bus. Nothing
but "go" and "come back" goes over it.
