# Changesets

Every PR that changes the app adds a changeset:

```sh
pnpm changeset
```

Pick the bump (`patch` for fixes, `minor` for features, `major` for breaking changes) and write
one line for the changelog. Commit the generated `.changeset/*.md` file with your PR.

When PRs with changesets land on `main`, the **version** workflow opens a "Version Packages" PR
that bumps `package.json` and updates `CHANGELOG.md`. Merging that PR builds the release.
Details: [README → Releasing](../README.md#releasing).
