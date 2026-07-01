# README Image Contract

`assets/` is the canonical directory for public README images.

This directory exists to make the contract explicit for operators and agents that
look under `docs/readme/` for README context after compaction. Do not mirror the
image assets here. Duplicating the README images would add another binary source
of truth and let the rendered README drift from the files that GitHub and local
clones actually load.

If an operator objective says README images should be "under `docs/readme`",
interpret that as a request to inspect this contract, not as permission to copy
or regenerate image files here.

The source of truth is the repository root `README.md` plus the exact bytes of
the image files it references. Every relative local image referenced there must
resolve to a real file under `assets/`, and its extension must match the
physical file signature. The verifier is:

```bash
bash scripts/check_readme_assets.sh
```

Run it before changing `README.md`, `assets/`, or `docs/readme/`, and before a
public mirror sync that includes any of those paths. If an image path moves,
update the root `README.md`, this contract, the verifier/test, and the public
mirror workflow documentation in the same change.
