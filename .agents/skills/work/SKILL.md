---
name: work
description: Take a GitHub issue of allock from analysis to a merged pull request - alternatives discussed with the user, implementation, headless shots of the lock and the greeter before and after, a checklist for what only a real session shows, review - when asked to work on an issue (/work <issue number>).
argument-hint: <issue number>
---

# Work on an issue

Take the issue the user named (in Claude Code it is `$ARGUMENTS`) to a pull request. The user
decides at three points: the approach, how it looks, and whether it works in a real session. In
between, work on your own. Talk to the user in the language they write in; write code, commits,
PRs and issue comments in English. The repository's rules are in `AGENTS.md`: follow them
throughout, the security ones first.

## 1. Understand

- `gh issue view <number> --comments`: the issue and what was already said on it. Its labels
  say what kind of work it is:
  - `look`: how the lock or the greeter appears changes;
  - `auth`: PAM, fprintd, greetd or faillock, anything on the way to an unlock: it takes the
    security review of step 5, and what happens on every failure is part of the design;
  - `greeter`: the greeter mode, its install and what it may read;
  - `config`: a config key, `install.sh` or the docs that describe them;
  - `tech-debt`: nothing visible changes: every scene must draw the same pixels.
  - `size:S|M|L`: how big it is.
- Read the code involved, the README and docs sections that describe it, and its tests.
- If the issue is out of date (already done, or no longer true), say so and propose to close
  it.
- If it needs something styletto lacks (a token, a component, a variant), say so: that part is
  an issue on styletto, not a number or a one-off widget here.
- For `look` and `tech-debt` work, take the "before" shots now (see step 4), on `main`.

## 2. Propose, then stop

Give 2-3 real alternatives (not one idea and two straw men). For each one, say what it costs
(code, config keys, dependencies, risk) and what it gives, and say which one you'd pick and why.
When the choice is visual, show it: quick prototypes drawn with `scripts/shot.sh`, never ASCII.
When the config changes, show the TOML a user would write. For `auth`, say for each alternative
what happens when each part fails (PAM errors, fprintd missing, greetd gone, a timeout): the
answer must always be "it stays locked". When it is not obvious, look at how other lock screens
and greeters solved it (swaylock, hyprlock, gtklock, GDM, SDDM, regreet).

Post the analysis as a comment on the issue (`gh issue comment`), then **end the turn and wait
for the user**. Don't write code before they choose. If the discussion changes the plan, add a
short comment with the decision.

## 3. Implement

- `git switch main && git pull --ff-only`, then a branch `<number>-<short-slug>`.
- Keep to the rules of `AGENTS.md`: nothing that can unlock on an error, no secret outside the
  authentication path, the look from styletto, a config change documented where it is
  described.
- Test what can be tested without a session: the config, the flow's logic, parsing, the
  decisions of the authentication path behind `Backend::Silent` or a fake.
- Keep the change to what the issue asks: anything else you notice goes to the final report,
  not into this branch.

## 4. Check, iterate

Run the checks of `AGENTS.md` and fix until they are clean.

Then look at the result. With `scripts/shot.sh`, draw every scene the change touches in the
same matrix before and after:

- `mode=dark` and `mode=light`;
- `material=solid`, `translucent` and `glass`, over the wallpaper (`lock/…`) and over the
  session's picture (`snapshot/…`) when the island or the background is involved;
- the corners of `density` × `shape` (compact+square, comfortable+round);
- a second theme (for example `theme=catppuccin`) when colours are involved;
- the states that matter: `lock/refused`, `lock/lockout`, `lock/power`, `greeter/sessions`.
  When the change has a state no scene shows, add the scene to `SHOT_SCENES` and `shot` in
  `src/ui/snapshots.rs`.

Use the same arguments before and after. Look at the shots yourself first. Then:

- iterate on your own while the result is objectively wrong: clipped or overflowing text,
  misaligned items, lost contrast, a regression in another scene, failing checks;
- taste is the user's: for `look` work, show the before/after and **wait for their ok** before
  going on;
- for `tech-debt`, prove that nothing visible changed: `magick compare -metric AE` gives 0 for
  every pair.

Never run `allock`, `allock --preview` or the greeter yourself. What the shots cannot show is
the user's to try: give them a short checklist of what to do and what they should see, with the
commands, for what the change touches. For example:

- `scripts/install.sh` (or `--greeter`), then `allock --preview`: the card, a wrong password, the
  right one;
- the real lock from the idle daemon or the key binding: wrong password, fingerprint, the
  lockout, suspend and resume with the card up;
- the greeter: log out, pick a user and a session, log in.

For `auth`, `greeter` and anything that runs only in a session, **wait for the user's report**
before step 6.

## 5. Simplify and review

Review the branch's whole diff against `main` twice: once for simplification (reuse of what the
crate or styletto already has, code that can go, clearer names), once for correctness (bugs,
regressions, numbers that should be styletto's, a config key undocumented). In Claude Code use
`/simplify` and then `/code-review`; elsewhere, use your tool's review command if it has one,
or do the two passes yourself. For `auth`, and anything that touches what is read or written
as root or as the greeter, add a security pass (`/security-review` in Claude Code): an error
that unlocks, a secret that leaks, a file the greeter reads from a user's home.

Act only on what really matters: bugs, regressions, rule breaks, code that will mislead the
next reader. Leave out style preferences and speculative refactors. Re-run the checks and the
shots after fixing; repeat the round only if the fixes were substantial (two rounds at most).

What the review finds in what this change brought in is fixed in this same pull request,
unless it is too large for it: then say so and propose an issue. Problems that were there
before and that the change only brought to light become issues, not part of this branch.

What you leave out that is worth doing later becomes an issue, after the user agrees:
`gh issue create --label tech-debt --label size:S|M|L` with a body that stands on its own
(where, what, why it was left). What belongs to styletto is an issue on styletto
(`gh issue create -R MalpenZibo/styletto`).

## 6. Pull request

- One commit (or a few that each make sense alone): a subject that says what changed, a body
  with one plain-English paragraph per topic. No trailers of any kind.
- `git push -u origin <branch>`, then `gh pr create` with: what changed and why, the
  alternative chosen, how it was checked (commands, scenes and settings shot, what the user
  tried in a session), any change to the config or to what users must do after updating,
  what was left out (with the new issues' numbers), and `Closes #<number>`. Screenshots can't
  be attached from the command line: describe them, the user has seen them.
- Wait for CI (`gh pr checks --watch`). If it fails, fix and push again.

## 7. Merge

Merge only with green CI and only when the user says so: `gh pr merge --squash
--delete-branch`, then `git switch main && git pull --ff-only`. Never push to `main` directly.
When the change needs the user to do something after updating (reinstall, a config key, a file
under `/etc` only they can write), end with the commands to do it.
