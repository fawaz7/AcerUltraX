AcerUltraX 0.1.2: your Acer laptop's Windows sound (DTS:X Ultra), natively on Linux.

**What's new**

- **5.1 and 7.1 sound keeps its dialogue.** With DTS off and in Compare, 5.1/7.1 sound (for example a movie in
  VLC) lost its centre channel, where most dialogue is. It is now mixed down exactly as Linux does without
  AcerUltraX, and Compare is fair on movies too (#2, thanks to u/Natethegreat9999 on Reddit).
- **Laptops with extra speakers:** woofers or rear speakers that show up as a 2.1 or 4.0 output now get sound.
- **Clean starts:** no more click at the start of 5.1/7.1 sound with DTS on, and apps that use PipeWire directly
  (mpv, some games) no longer lose the first 20 ms of every sound (#3).
- **More accurate Compare:** it allows for DTS's own delay, follows DTS's loudness control within about a second,
  and is right the moment you switch it on (#3).
- **Speakers sleep when nothing plays,** as without AcerUltraX, instead of staying powered all the time.
- Apps you have sent to another output, and apps marked not to be moved, are no longer taken over.

**Install**

- **Debian 13, Ubuntu 24.04 and newer:** add the apt repository (see the [README](https://github.com/fawaz7/AcerUltraX#install)), or download `acerultrax_*_amd64.deb` below and run `sudo apt install ./acerultrax_*_amd64.deb`.
- **Any other distribution, no package manager:** download `acerultrax-*-x86_64-linux.tar.gz`, unpack it and run `./install.sh`.
- **Arch-based:** `packaging/aur/PKGBUILD` in the repository.

Then open **AcerUltraX** and click **Download and set up**.

**Everything in AcerUltraX**

- Acer's sound modes (Music, Movie, Game 1–3, Voice, Custom, Off), matching Windows bit for bit.
- A profile per output: laptop speakers, 3.5 mm headphones, each Bluetooth device.
- Over 500 headphone models, spatial sound for 5.1/7.1, your own profiles with a 10-band equaliser.
- Top-bar icon with four profiles of your choice; one-click setup from Acer's servers.

Verified on the Acer Predator Helios 16 (PH16-71). Other Acer laptops with DTS:X Ultra are experimental: please
[report how it works](https://github.com/fawaz7/AcerUltraX/issues).

`SHA256SUMS` lists the checksums of the files below.
