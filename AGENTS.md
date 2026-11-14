# allock: notes for coding agents

A lock screen and greetd greeter for Wayland, drawn as an island, in Rust, on
[Guido](https://github.com/MalpenZibo/guido) and the
[styletto](https://github.com/MalpenZibo/styletto) design system. [README.md](README.md) is the
documentation for users, [docs/SETUP.md](docs/SETUP.md) how to set it up, and
[docs/HANDOFF.md](docs/HANDOFF.md) the protocol with a bar: read what a change touches before
changing it, and keep it true.

## Layout

- `src/main.rs`: the modes (lock, `--preview`, `--greeter`) and the `Screen` each one builds.
- `src/config.rs`: the config file, `[appearance] import` included.
- `src/services/auth`: PAM, fprintd and greetd, behind one `Auth`.
- `src/services`: logind, idle, the snapshot of the session, the handoff with a bar, battery,
  power, the clock.
- `src/system`: faillock, accounts, the greeter's last login.
- `src/ui`: the flow (seed, rest, awake, gone) and what draws it: island, card, field, clock,
  background. `snapshots.rs` draws it headless.
- `pam/allock`: the lock's PAM policy. `scripts/install.sh` installs everything, `scripts/shot.sh`
  and `scripts/demo.sh` draw it.

## Rules

A lock screen is trusted to stay shut. Before anything else:

- An error never unlocks. A PAM, fprintd, greetd or compositor failure leaves the session locked
  (or the greeter at its login), and says so on the card or in the log.
- A secret never leaves the authentication path: not into a log line, a `Debug` output, an error
  message or a buffer that is not wiped (`Secret`, `zeroize`).
- The handoff with a bar carries "go" and "come back", never a credential, an attempt or a
  verdict; the lock is never held back for it, or for any animation: everything it waits on
  has a timeout.
- The greeter runs as the `greeter` user: nothing it reads may be in a user's home.

And for the rest:

- The look is styletto's: every length, corner, size and colour comes from its scales and the
  `Ui`. When a token or a component is missing, it is added to styletto (an issue there), not
  written as a number here.
- A screen must look right in light and dark, with every `Material`, `Density` and `Shape`, and
  with more than one theme.
- A config key added or changed updates, in the same change, `config.example.toml` (every key,
  with its default), the README section that describes it and, when the greeter needs it,
  `scripts/install.sh`. The example config must keep parsing to the defaults (a test checks).
- allock runs without ashell and without any bar: what is for a bar stays optional.

## Style

Follow the code around you: comments say why, in plain English; `cargo fmt` decides the
layout. Prefer a small, direct change over a new abstraction.

## Checks

What CI runs (`.github/workflows/ci.yml`); all must pass:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

## Seeing a change

Never run `allock`, `allock --preview` or the greeter yourself: they cover every screen, even
one shared in a call, and a real lock can only be undone with the user's password. Draw the
screens headless into PNGs and look at them:

```sh
scripts/shot.sh --list                                 # every scene, with what it shows
scripts/shot.sh lock/card /tmp/card.png                # the lock's card, the default look
scripts/shot.sh greeter/sessions /tmp/s.png mode=light theme=catppuccin material=glass
```

`key=value` are the keys of the `[appearance]` table (see the README). The clock (09:41), the
machine and the accounts are fixed, so the same arguments give the same pixels and a
before/after pair compares exactly: `magick compare -metric AE before.png after.png null:`
prints how many pixels differ (0 means none). It needs a GPU adapter; the first run builds in
release. A scene not in the list is added to `SHOT_SCENES` and `shot` in
`src/ui/snapshots.rs`.

What headless shots cannot show (PAM, the fingerprint reader, suspend and resume, the session
lock itself, greetd starting a session) is tried by the user, from a checklist you give them.

## Git

- `main` changes only through pull requests with green CI; never push to it.
- Branches are `<issue>-<short-slug>`. Commits and PRs are in English; a commit subject says
  what changed, the body explains why. No trailers (no `Co-Authored-By`), no "Generated with"
  footers.
- PRs are squash-merged.
