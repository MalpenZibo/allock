# Setup

## Install

```sh
scripts/install.sh             # lock screen (the default)
scripts/install.sh --greeter   # greeter
scripts/install.sh --all       # both
```

Run it as your own user, it asks for sudo when it needs root. It never edits
greetd's config or yours.

| File | Mode |
|---|---|
| `/usr/local/bin/allock` | all |
| `/etc/pam.d/allock` | lock screen |
| `/etc/greetd/allock.toml`, the greeter's config | greeter |
| `/etc/greetd/allock-niri.kdl`, the niri that hosts the greeter | greeter |
| `/usr/local/share/allock/greeter-wallpaper.*` | greeter |
| `/var/cache/allock/`, the last user and session | greeter |

The greeter runs as the `greeter` user and cannot read your home, so the script
copies what it needs: the `[appearance]` table of your allock config, and the
keyboard layout and cursor theme of your niri config. Run it again with
`--force` after changing them.

The greeter's wallpaper is one fixed picture: `GREETER_WALLPAPER` at the top of
the script, or `--wallpaper FILE`.

## Lock screen

Try it first without locking anything:

```sh
allock --preview
```

allock locks when it starts and exits once unlocked. Bind it to a key, niri for
example:

```kdl
binds {
    Mod+Escape allow-inhibiting=false { spawn-sh "pidof allock || allock"; }
}
```

Use the same `pidof allock || allock` in your idle daemon's lock, suspend and
lid actions, so a second lock never starts over the first.

## Greeter

In `/etc/greetd/config.toml`:

```toml
[default_session]
command = "systemd-cat -t allock-greeter niri -c /etc/greetd/allock-niri.kdl"
user = "greeter"
```

`systemd-cat` keeps niri's output off the console and in the journal
(`journalctl -t allock-greeter`). allock starts your session the same way
(`journalctl -t niri-session`), `session_output = "Console"` turns that off.

`[initial_session]` in greetd's config is autologin: remove it to see the
greeter at every boot. If you keep it, lock your session at startup instead.

The greeter's options go in `/etc/greetd/allock.toml` under `[greeter]`:

| Option | |
|---|---|
| `default_session = "Niri"` | always start on this session, instead of the last one used |
| `other_user = false` | drop "Other…" from the user picker |
| `user = "name"` | preselect this account |
| `remember = false` | forget the last user and session |
| `[[greeter.session]]` | your own session list, see the example config |

For an avatar, put a square picture at `/var/lib/AccountsService/icons/<user>`.

### Before you reboot

Check the greeter from a text console first. Switch to one with Ctrl+Alt+F2,
log in and run:

```sh
systemctl --user stop niri.service
sudo systemctl restart greetd
```

Stop your niri first: it runs under your user's systemd, so restarting greetd
leaves it running and `niri-session` then refuses to start a second one.
Restarting greetd ends the sessions it started, so save your work.

If the greeter does not come up, `journalctl -b -u greetd` says why. Put the
old `[default_session]` back and restart greetd again.
