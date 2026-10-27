# Arch / Omarchy packaging

`PKGBUILD` builds one package, `pipedeck`: `/usr/bin/{pipedeckd,pipedeck,pipedeck-ui}`, the
systemd user unit, the D-Bus activation file, the `pipedeck-ui` launcher entry and icon, the
bundled EQ presets in `/usr/share/pipedeck/presets/`, LICENSE and README (SPEC-UI §7).
`pipedeckd.service` and `dev.pipedeck.Daemon.service` here are the `/usr/bin` variants of the
`%h/.local/bin` units one directory up, which stay for `install.sh`.

## Build and install locally

```bash
cd packaging/arch
makepkg -fsi        # build, pull missing deps, install (asks for sudo)
```

Where sudo wants a password the session cannot give, build without the dependency check and
install by hand:

```bash
makepkg -fd
sudo pacman -U pipedeck-1.4.0-1-x86_64.pkg.tar.zst
```

`ui/bin/install` runs `makepkg -fsi` here. Then, per user:

```bash
systemctl --user enable --now pipedeckd
```

`source=` is the GitHub release tarball for `v$pkgver`. Until that tag exists, build from the
working tree by putting a tarball of that name next to the PKGBUILD; makepkg uses a local file
instead of downloading:

```bash
tar czf packaging/arch/pipedeck-1.4.0.tar.gz --transform 's,^,pipedeck-1.4.0/,' \
  --exclude=target --exclude=ui/build --exclude=.git \
  --exclude='packaging/arch/*.tar.gz' --exclude='packaging/arch/pkg' --exclude='packaging/arch/src' \
  -C "$(git rev-parse --show-toplevel)" .
```

## User-level alternative (no root)

```bash
./install.sh --no-extension   # daemon + CLI to ~/.local/bin, user unit, D-Bus file, presets
ui/bin/install-user           # pipedeck-ui to ~/.local/bin, launcher entry, icon
```

Remove these before installing the package, or the `~/.local` copies shadow it.

## Releasing and omarchy-pkgs

1. Bump the workspace version in `Cargo.toml` and `pkgver` here, reset `pkgrel=1`, tag
   `v$pkgver`, publish the GitHub release.
2. `updpkgsums` here, to replace `sha256sums=('SKIP')` with the release tarball's sum.
3. An `omacom/omarchy-pkgs` PR is these three files, unchanged:

   | here | there |
   |---|---|
   | `PKGBUILD` | `pkgbuilds/pipedeck/PKGBUILD` |
   | `pipedeck.install` | `pkgbuilds/pipedeck/pipedeck.install` |
   | `omarchy-pkgs/package.json` | `pkgbuilds/pipedeck/.omarchy/package.json` |

   The `upstream.watch` on GitHub Releases means later releases arrive there without another PR.
   Check it with `python helpers/upstream-watch.py check pkgbuilds/pipedeck` and build with
   `bin/build --package pipedeck` (clean container) in that repo.

Publishing there is out of scope until Jan says so (outward-facing).
