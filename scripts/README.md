# Maintainer scripts

| Script | What it does |
|---|---|
| `check.sh` | Formatting, lints and every test (reference tests too when the Acer files are set up). Run before each commit. |
| `snapshot.sh <page> <out.png>` | Screenshot a page of the app built from this checkout, without touching the running app. |
| `security-pass.sh <branch>` | Lists every change of a contributor branch that needs a close security look (scripts, CI, dependencies, network, unsafe code, privileges, secrets). |
| `nix-build.sh [acerultrax-headless]` | Builds the Nix package in a NixOS container (no Nix install needed); the Nix store is kept in the Docker volume `acerultrax-nix`. |
| `release.sh X.Y.Z` | Releases the tested `preview` branch: bump, check, fast-forward `main`, tag, push, then confirms the GitHub release and the apt repository. |

Work happens on the `preview` branch; `main` is always the latest release.
