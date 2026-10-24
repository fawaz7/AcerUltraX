AcerUltraX 0.1.3: your Acer laptop's Windows sound (DTS:X Ultra), natively on Linux.

> **Update to this version the usual way once more:** from 0.1.3 on, AcerUltraX checks for updates itself and
> installs them from the About page.

**What's new**

- **Acer's newest DTS driver (2026).** Setup now uses it: sound tuning for 278 Acer laptops instead of 162,
  including the tunings Acer improved since 2023 (#5). Laptops Acer re-tuned get a "Newer sound from Acer" button
  in Settings. The DTS engine itself sounds exactly the same in every version; only Acer's per-laptop tuning changed.
- **Many more laptops can be set up.** About a third of Acer's DTS laptops have five sound modes instead of eight
  and could not be set up before. They now work.
- **552 headphone models** from Acer's 2026 DTS app, including HyperX Cloud III, Cloud III S Wireless and Cloud
  Alpha. Headphones you already picked stay picked.
- **Updates from inside the app.** The About page tells you when a new version is out and installs it with one
  click (download and apt installs). Your settings and profiles are kept.
- Newer Acer sound hardware (Senary, SoundWire, Realtek USB audio) is recognised (experimental).
- Fixes: setting up the driver again no longer drops the headphone models; the "Star on GitHub" button is readable.

**Install**

- **Debian 13, Ubuntu 24.04 and newer:** add the apt repository (see the [README](https://github.com/fawaz7/AcerUltraX#install)), or download `acerultrax_*_amd64.deb` below and run `sudo apt install ./acerultrax_*_amd64.deb`.
- **Any other distribution, no package manager:** download `acerultrax-*-x86_64-linux.tar.gz`, unpack it and run `./install.sh`.
- **Arch-based:** `packaging/aur/PKGBUILD` in the repository.

Then open **AcerUltraX** and click **Download and set up**.

**Everything in AcerUltraX**

- Acer's sound modes (Music, Movie, Game 1–3, Voice, Custom, Off), matching Windows bit for bit.
- A profile per output: laptop speakers, 3.5 mm headphones, each Bluetooth device.
- 552 headphone models, spatial sound for 5.1/7.1, your own profiles with a 10-band equaliser.
- Top-bar icon with four profiles of your choice; one-click setup from Acer's servers; one-click updates.

Verified on the Acer Predator Helios 16 (PH16-71). Other Acer laptops with DTS:X Ultra are experimental: please
[report how it works](https://github.com/fawaz7/AcerUltraX/issues).

`SHA256SUMS` lists the checksums of the files below.
