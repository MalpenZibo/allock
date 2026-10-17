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
  <img src="assets/screenshots/lock.webp" alt="The lock screen" width="100%">
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
- A handoff with [ashell](https://github.com/MalpenZibo/ashell): the bar folds
  its island into the same dot before the lock appears, and grows back out of
  it after the unlock
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
[`config.example.toml`](config.example.toml) to `~/.config/allock/config.toml`:
every key is optional and shown with its default.

The layout is fixed. The config chooses colours, wallpaper, clock format, the
seed's size and place, which indicators appear and which monitors show the
controls.

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

The island has one smallest state, the **seed**: a dot of `[island.seed] size`
pixels, `top` pixels from the top edge. Everything starts and ends there. When
the lock takes the screen the compositor cuts from the session to the lock
surface, and no client can animate that cut; when the dot is in the same place
on both sides, the one thing the eye follows does not move.

Before locking, allock asks the bar to fold its island into the seed over
D-Bus (`org.ashell.Island1.PrepareHandoff("lock")` on the session bus), draws
the seed in its first frame, and grows it. Unlocking does the reverse: the card
folds into the seed, the session comes back with the bar still a seed, and
`ResumeAfterHandoff()` lets it grow. logind's `LockedHint` is kept too, so a bar
restarted mid-lock knows to wait as the seed.

The background is the only thing the compositor still cuts. With
`[background] source = "Snapshot"` not even that: once the bar is the seed,
allock takes a picture of every output through `wlr-screencopy`, starts the
lock on it sharp and blurs it, and sharpens it again under the seed before
handing back. The pictures stay in memory and are never written anywhere; an
output that cannot be captured, or a compositor that takes longer than half a
second, gets the wallpaper.

The handoff is optional: without a bar allock locks at once, and a bar that
does not answer within `[handoff] timeout_ms` is not waited for. Nothing but
"fold" and "resume" goes over it. The greeter does the same on its side of the
compositor switch: it folds into the seed over the wallpaper, sharpened, which
is the picture the session starts on.
