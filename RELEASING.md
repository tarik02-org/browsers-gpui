# Releasing

Release Please maintains a pull request named `chore: release <version>` after
changes land on `main`. Merge that pull request to create the tag and GitHub
release. The existing CI workflow then builds and uploads the Linux, macOS, and
Windows artifacts.

Versions use `Y.M.DN` calendar versioning. The patch component contains the day
and a two-digit daily release number:

- The first release on August 14, 2026 is `2026.8.1400`.
- The next release that day is `2026.8.1401`.
- The first release on September 1, 2026 is `2026.9.100`.

The release date uses the `Europe/Kyiv` timezone. A scheduled workflow refreshes
an open release pull request each hour so its version follows the current date.
The daily release number is derived from existing tags and supports values from
`00` through `99`.

The first workflow run creates a `0.7.4` tag at the last commit before release
automation was introduced. Release Please uses it as the baseline for the first
generated changelog.

The repository setting **Allow GitHub Actions to create and approve pull
requests** must be enabled. No release token or registry token is required.
