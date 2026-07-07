# Changelog

All notable changes to this project are documented here, following
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) conventions.

<!--
How to cut a release:

1. As you work, add entries under the `## [Unreleased]` section below, using
   the standard sub-headings (Added / Changed / Fixed / Removed / Security) as
   needed — omit any sub-heading you have nothing to say under.
2. When you're ready to release, rename `## [Unreleased]` to
   `## [v<major>.<minor>.<patch>] - <YYYY-MM-DD>` (matching the tag you're
   about to push exactly, e.g. `## [v1.2.3] - 2026-07-07`), and add a fresh,
   empty `## [Unreleased]` section above it for future work.
3. Commit that change to `master`, then tag the resulting commit with a
   `v`-prefixed version: `git tag v1.2.3 && git push origin v1.2.3`.
4. .github/workflows/release.yml triggers on that tag, builds the release
   bundles, and extracts the `## [v1.2.3]` section verbatim as the GitHub
   Release body — so the heading format above must match exactly, or the
   workflow fails fast with an error rather than publishing a Release with no
   notes.
-->

## [Unreleased]
