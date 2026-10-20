# Working on dev, publishing to main

`main` is the public face of Utter: code, packaging, the README. `dev` is where all work happens
and carries everything else: `docs/`, `DESIGN.md`, `CLAUDE.md`, `.claude/`, this folder. Both
branches are readable on GitHub, so neither may hold a path on the maintainer's machine, an address
on a private network, an email address or a credential. `dev` may name the maintainer and the
private projects around Utter; `main` may not.

## Daily

Work, commit and push on `dev`. CI runs on both branches.

## Publishing to main (only when the maintainer says so)

```
uv run --no-project python dev/release.py selftest    # the rules still catch what they should
uv run --no-project python dev/release.py scan        # everything the next publish would reject
uv run --no-project python dev/release.py fix         # the mechanical rewrites in patterns.toml
uv run --no-project python dev/release.py publish --dry-run
uv run --no-project python dev/release.py publish --push -m "Utter 0.1.11: ..."
```

`publish` needs a clean working tree on `dev` and a clean scan. It builds main's next tree from
dev's last commit, removes the private paths, points links to them at the dev branch on GitHub,
scans that tree and the commit message again, and adds one commit on top of main. It is a tree
copy, not a merge: main's history is the list of publishes.

## The rules

`patterns.toml` is committed and generic. Strings that would leak by being written down (a
hostname, an address, a folder name) go in `patterns.local.toml`: same format, git-ignored. Keep a
copy in your backups; a fresh clone has only the generic rules.

- scope `all`: nowhere in the repository (home-folder paths, private addresses, emails, secrets).
- scope `main`: fine on dev, never on main (the maintainer's name outside attribution, private
  infrastructure and projects).
- A rule that fires wrongly gets an `allow` regex and a `bad`/`ok` sample; `selftest` checks them.
- Known and accepted: the attribution `Tai Duc Nguyen` (authors, publisher, licence), the GitHub
  handle in URLs, benchmark hardware model names, the `Utter_dev` data folder name.

## Releases

Build installers from `main` after a publish, so what ships is what is public. A change to
`Casks/utter.rb` goes through dev and a publish like any other.
