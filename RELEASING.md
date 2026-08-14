# Releasing

Release Please maintains a pull request named `chore: release <version>` after
changes land on `main`. Merge that pull request to create the tag and GitHub
release. The existing CI workflow then builds and uploads the Linux, macOS, and
Windows artifacts.

The release changelog comes from conventional commit types. Pull request titles
must use a conventional type, for example `feat: add tab groups`, `fix: restore
saved windows`, or `build: update the Linux package`. The supported types are
`feat`, `fix`, `perf`, `refactor`, `docs`, `deps`, `build`, `ci`, and `chore`.
The pull request title workflow enforces this format. Scopes such as
`feat(tabs): add groups` and breaking changes such as `refactor!: remove legacy
storage` are also valid.

Versions use `Y.M.DN` calendar versioning. The patch component contains the day
and a two-digit daily release number:

- The first release on August 14, 2026 is `2026.8.1400`.
- The next release that day is `2026.8.1401`.
- The first release on September 1, 2026 is `2026.9.100`.

The release date uses the `Europe/Kyiv` timezone. A scheduled workflow refreshes
an open release pull request each hour so its version follows the current date.
The daily release number is derived from existing tags and supports values from
`00` through `99`.

The `bootstrap-sha` in `release-please-config.json` marks the last commit before
release automation. Release Please uses it as the cutoff for the first generated
changelog. The workflow does not create a tag at that commit.

The repository setting **Allow GitHub Actions to create and approve pull
requests** must be enabled. No release token or registry token is required.
