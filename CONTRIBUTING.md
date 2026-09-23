# Contributing

Bug reports and PRs are welcome. For bugs, the [issue template](.github/ISSUE_TEMPLATE/bug_report.md) asks for what we need.

## Build from source

You need Node 20+, pnpm, and stable Rust. On Linux you also need `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev`.

```sh
pnpm install
pnpm tauri dev              # also builds the adshook sidecar
cd src-tauri && cargo test --workspace
pnpm tauri build            # installers in src-tauri/target/release/bundle/
```

CI runs `cargo fmt --check`, `cargo clippy -- -D warnings` and the tests on macOS, Windows and Ubuntu. Run them before you push.

## Code layout

| Path | What's there |
|---|---|
| `src-tauri/src/lib.rs` | The main loop, commands and menu actions |
| `src-tauri/src/decide.rs` | The hold/release rules (pure, tested) |
| `src-tauri/src/tray.rs` | The tray menu, icon states and labels |
| `src-tauri/src/alerts.rs` | Needs-you, stuck, long-turn and error alerts |
| `src-tauri/src/stats.rs` | Daily activity files for the Activity tab |
| `src-tauri/src/agents.rs` | Agent integrations and session scanning |
| `src-tauri/src/power/` | One API, one file per OS |
| `src-tauri/hook/` | `adshook`, the helper every agent hook calls |
| `src/` | The React settings window |

**Icons:** `python3 scripts/make-icons.py && pnpm tauri icon app-icon.png`

## Releasing

Versioning is automated with [Changesets](https://github.com/changesets/changesets). Nobody edits `version` in `package.json` by hand; Tauri reads the app version from there.

1. **Add a changeset to each PR** that changes the app:
   ```sh
   pnpm changeset
   ```
   Pick `patch` for fixes, `minor` for features or `major` for breaking changes, then write one changelog line. Commit the new `.changeset/*.md` file with the PR. Docs-only and CI-only PRs don't need one.
2. **Merge to `main`.** The **version** workflow opens (or updates) a **"chore: version packages"** PR. That PR bumps `package.json` from all pending changesets and writes `CHANGELOG.md`, with links to the PRs.
3. **Merge the version PR.** The version workflow sees the new version and runs the **release** workflow. It builds four installers (macOS Apple silicon and Intel, Windows, Linux) into a **draft** GitHub release tagged `vX.Y.Z`. This takes about 20 minutes.
4. **Test the installers, then publish the draft** under *Releases*.

**Pre-releases:** push a tag by hand to build a test release from any commit on `main`. A tag with a `-` in it is marked as a pre-release.
```sh
git tag v0.2.0-rc.1 && git push origin v0.2.0-rc.1
```

**If a release job fails:**
1. Fix it in a PR.
2. Delete the draft and its tag: `gh release delete vX.Y.Z --cleanup-tag`.
3. Push the tag again by hand, from the fixed commit.

**CI on the version PR:** GitHub doesn't run workflows on PRs created with the built-in token, so the version PR has no checks and needs an admin merge. To get CI on it, add a fine-grained personal access token as the repository secret `CHANGESETS_TOKEN`, with **Contents** and **Pull requests** set to *read and write* for this repo.

**Signing** can be added later with no code changes. Add the `APPLE_*` secrets listed in `.github/workflows/release.yml`.
