AcerUltraX 0.1.4: your Acer laptop's Windows sound (DTS:X Ultra), natively on Linux.

> **On 0.1.2 or older?** Update this once the usual way: from 0.1.3 on, AcerUltraX checks for updates itself and
> installs them from the About page.

**What's new**

- **NixOS:** a Nix flake and a Home Manager module (#4). Thanks to @gettbitgirl!
- **Predator Helios 16 PH16-72** confirmed working by its owner (#5).
- Buttons visibly react when you point at them and click them.
- Links on the About page fall back to the default browser when the desktop's link handler fails.

**Known issue**

- **NixOS with Home Manager:** leave "Start with my session" and "Top bar icon" alone for now; Home Manager starts
  AcerUltraX for you. A proper fix is on the way.

**Install**

- **Debian 13, Ubuntu 24.04 and newer:** add the apt repository (see the [README](https://github.com/fawaz7/AcerUltraX#install)), or download `acerultrax_*_amd64.deb` below and run `sudo apt install ./acerultrax_*_amd64.deb`.
- **Any other distribution, no package manager:** download `acerultrax-*-x86_64-linux.tar.gz`, unpack it and run `./install.sh`.
- **Arch-based:** `packaging/aur/PKGBUILD` in the repository.
- **NixOS / Home Manager:** the repository's flake (see the README).

Then open **AcerUltraX** and click **Download and set up**.

**Everything in AcerUltraX**

- Acer's sound modes (Music, Movie, Game 1–3, Voice, Custom, Off), matching Windows bit for bit.
- A profile per output: laptop speakers, 3.5 mm headphones, each Bluetooth device.
- 552 headphone models, spatial sound for 5.1/7.1, your own profiles with a 10-band equaliser.
- Top-bar icon with four profiles of your choice; one-click setup from Acer's servers; one-click updates.

Verified on the Acer Predator Helios 16 (PH16-71). Other Acer laptops with DTS:X Ultra are experimental: please
[report how it works](https://github.com/fawaz7/AcerUltraX/issues).

`SHA256SUMS` lists the checksums of the files below.
