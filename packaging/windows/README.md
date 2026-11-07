# Windows packaging

No Windows installer is built yet: there is no `.wxs`, no `package.ps1` and no signing step here.
The directory exists because the suite's `packaging-lint` workflow runs

```powershell
Get-ChildItem packaging/windows/*.ps1
```

without an existence guard (tracked as
[craft-common-actions#145](https://github.com/storytold/craft-common-actions/issues/145)). On a
checkout with no `packaging/windows/`, that glob is an error under the runner's
`$ErrorActionPreference = 'Stop'`, so a pull request that touches `assets/app-icon/` fails a lint
about a package that does not exist. An empty directory makes the glob return nothing, which is
what it should do.

When Windows packaging lands, put `package.ps1`, the WiX source and the signing scripts beside this
file; the shared release workflow in `storytold/craft-common-actions` calls them.
