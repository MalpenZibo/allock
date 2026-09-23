# Setting allock up

`scripts/install.sh` puts the files in place. What it does not do is change
how your machine logs in and locks: those are your configs, and the steps are
below.

## 1. Install

```sh
scripts/install.sh             # lock screen only
scripts/install.sh --greeter   # lock screen and greeter
```

Run it as yourself; it asks for sudo where it needs root.

| What | Where | Mode |
|---|---|---|
| the binary | `/usr/local/bin/allock` | both |
| the PAM policy (see the README) | `/etc/pam.d/allock` | lock |
| the greeter's allock config | `/etc/greetd/allock.toml` | greeter |
| the niri config hosting the greeter | `/etc/greetd/allock-niri.kdl` | greeter |
| one fixed wallpaper, copied | `/usr/local/share/allock/greeter-wallpaper.*` | greeter |
| your ashell config, copied for its colours | `/etc/greetd/allock-ashell.toml` | greeter |
| the greeter's memory of the last login | `/var/cache/allock/` | greeter |

The greeter runs as the `greeter` user, which cannot read your home, so the
script copies what it needs out of it: your ashell config, and the keyboard
layout and cursor theme from your niri config. Re-run it with `--force` after
changing any of those.

The greeter's wallpaper is one fixed picture, copied once, rather than
whatever your session's wallpaper currently is — a rotating library would
have to be kept in step in two places. It is `GREETER_WALLPAPER` at the top of
the script, or `--wallpaper FILE`; when that file is missing the script falls
back to `[background] path` in your allock config.

Before going further, try the lock screen without locking anything:

```sh
allock --preview
```

It exits when your password is accepted.

## 2. Lock screen

allock locks when it starts and exits when unlocked, like hyprlock and
swaylock, so wherever you start one of those, start `allock` instead.

**A key binding** — niri:

```kdl
binds {
    Mod+Escape allow-inhibiting=false { spawn "allock"; }
}
```

**Idle and lid** — whatever idle daemon you run. Guard it so a second lock is
not started over the first (the compositor would refuse it anyway):

```sh
pidof allock || allock
```

- hypridle: `lock_cmd = pidof allock || allock` in `general`, and
  `before_sleep_cmd = loginctl lock-session` to lock before suspending.
- swayidle: `timeout 300 'pidof allock || allock'` and
  `before-sleep 'pidof allock || allock'`.
- stasis: `command "allock"` in the `lock_screen` step of each plan, and
  `lid_close_action "allock"`.

**Locking at login.** If your session starts a locker at startup
(`spawn-at-startup "hyprlock"`), that exists because greetd logs you in
without asking (`[initial_session]`). With allock as the greeter the greeter
is what asks, and the startup lock should go; keep it only if you keep
autologin.

## 3. Greeter

In `/etc/greetd/config.toml`:

```toml
[default_session]
command = "systemd-cat -t allock-greeter niri -c /etc/greetd/allock-niri.kdl"
user = "greeter"
```

`systemd-cat` sends what the greeter's niri and allock print to the journal
(`journalctl -t allock-greeter`) instead of the console. greetd runs the
greeter on its VT, so without it niri's log is what you read on tty1 before the
greeter draws and again while it hands over to your session.

The session you log in to gets the same treatment from allock itself: it is
started through `systemd-cat`, so what its start script prints lands in the
journal (`journalctl -t niri-session`) rather than on the console for a moment
before the desktop appears. `session_output = "Console"` turns that off.

Options for the greeter itself — `default_session`, `other_user`, `user`,
`session_output` — go in `/etc/greetd/allock.toml` under `[greeter]`, not in
greetd's config.

**`[initial_session]`** is autologin: greetd runs it once at boot without
asking, and shows `default_session` — the greeter — only after you log out.
Remove the section to see the greeter at every boot. Keep it if you prefer to
land in your session and lock it at startup, as above.

The greeter lists the accounts people log in to in a user picker, with
"Other…" for any other name, and preselects the last one used. To pick a
session once and for all, set `default_session = "Niri"` under `[greeter]` in
`/etc/greetd/allock.toml`; without it the last session used comes back.

For an avatar, put a square picture at
`/var/lib/AccountsService/icons/<your user>`: the greeter cannot read
`~/.face` in a private home.

The greeter lists the sessions installed under `/usr/share/wayland-sessions`
(and `/usr/share/xsessions`). To offer a different list, add
`[[greeter.session]]` entries to `/etc/greetd/allock.toml`; the example config
shows the shape.

### Before you reboot

A greeter that fails to start leaves you at a blank screen, so check it from a
text console first. From your session, switch to one with Ctrl+Alt+F2, log in,
and:

```sh
systemctl --user stop niri.service   # the session you are leaving
sudo systemctl restart greetd
```

Stop the old niri first. `niri.service` runs under your user's systemd, not
inside greetd's session, so restarting greetd leaves it running, and
`niri-session` then refuses to start a second one ("A niri session is already
running"). A reboot starts clean.

greetd takes over its VT (see `vt` in its config) and shows the greeter. If
something is wrong, `journalctl -b -u greetd` says what; put `[default_session]`
back to what it was (for example `agreety --cmd niri-session`) and restart
greetd again.

Restarting greetd ends the sessions it started — including the one you are
in, if greetd logged you into it. Save your work first.
