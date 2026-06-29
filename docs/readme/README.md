# README Image Contract

`assets/` is the canonical directory for public README images.

This directory exists to make the contract explicit for operators and agents that
look under `docs/readme/` for README context after compaction. Do not mirror the
PNG assets here. Duplicating the README images would add another binary source
of truth and let the rendered README drift from the files that GitHub and local
clones actually load.

The source of truth is the image paths in the repository root `README.md`.
Every relative local image referenced there must resolve to a real file under
`assets/`. If an image path moves, update the root `README.md`, this contract,
and the README asset verifier in the same change.
