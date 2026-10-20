<p align="center"><img src="data/icons/hicolor/256x256/apps/io.github.fawaz7.AcerUltraX.png" width="128" alt=""></p>

<h1 align="center">AcerUltraX</h1>

<p align="center">Your Acer laptop's Windows sound, on Linux.</p>

Acer gaming laptops ship with **DTS:X Ultra** on Windows: per-mode tunings (Music, Movie, Voice, Game, Custom
Audio) made for the laptop's own speakers and for headphones. On Linux the same speakers sound flat and thin.
AcerUltraX runs Acer's original DTS engine natively on Linux, without Wine, and processes your audio the way
Windows does, sample for sample.

> **Status: beta.** DTS runs natively and matches Windows bit for bit in offline tests; live sound, the app,
> profiles, headphone models and Windows' spatial sound work.

## Install

Arch Linux, CachyOS and other Arch-based systems: `yay -S acerultrax-git`. Then open **AcerUltraX** and click
**Download and set up**: it fetches your laptop's DTS files from Acer and is ready in about a minute. See the **[user guide](docs/USER_GUIDE.md)**.

## Supported laptops

| Laptop | Sound chip ID | Status |
|---|---|---|
| Acer Predator Helios 16 PH16-71 | `1025166C` | **Supported**: verified against Windows |
| Other Acer laptops with DTS:X Ultra | see `acerultrax-cli detect` | **Experimental** |

Acer's DTS package contains tunings for about 160 Acer models. They should work the same way, but nobody has
checked them yet. If you have one, please try it and
[open an issue](../../issues) with the output of `acerultrax-cli detect` and how it sounds. Reports of "it works"
are just as useful as bug reports.

## How it works

- AcerUltraX contains **no DTS or Acer files**. On first start it downloads Acer's own driver package for your
  laptop straight from Acer's download server (checked against known checksums), keeps the few files it uses and
  deletes the rest. It loads the DTS engine from them with a small built-in loader for Windows libraries.
- It applies the same factory settings Windows does, from Acer's tuning files for your laptop.
- It contains no licence checks. The DTS engine checks none on Linux either: on Windows, licensing lives in
  other parts of the DTS software, which AcerUltraX does not use.

Requirements: a 64-bit x86 (x86_64) Acer laptop, Linux with PipeWire and WirePlumber (`wpctl`).

## Try it (developers)

```sh
cargo build --release
./target/release/acerultrax-cli detect          # shows your sound chip and whether it is supported
```

Download Acer's DTS files from Acer and set them up (`--no-extras` skips the 386 MB DTS app that adds headphone
models and spatial sound). Already downloaded them? `acerultrax-cli import ~/Downloads/XPERI*DTS*.zip`.

```sh
./target/release/acerultrax-cli setup
./target/release/acerultrax-cli status
./target/release/acerultrax-cli render song.wav song-dts.wav --mode movie --output-device speakers
```

Run DTS on your system's sound:

```sh
./target/release/acerultrax-daemon              # or: acerultrax-cli autostart on
./target/release/acerultrax-cli mode movie      # music, movie, voice, game1-3, custom audio, off
./target/release/acerultrax-cli compare on      # same loudness as plain sound, for a fair comparison
```

Like Windows' DTS app, AcerUltraX can tune for your headphone model (over 500 models from Acer's and DTS's
databases; Bluetooth and USB headphones are recognised automatically):

```sh
./target/release/acerultrax-cli headphones hyperx                   # search the models
./target/release/acerultrax-cli headphone "hyperx cloud core" --device @headphones   # 3.5 mm jack
```

AcerUltraX becomes your default output. Pick the device you want in your desktop's sound settings as usual and
AcerUltraX follows it; each device remembers its own mode. The volume is applied to the device after DTS, as
on Windows.

`render` processes a WAV file (mono, stereo, 5.1 or 7.1) offline, so you can listen to DTS without changing
anything on your system. DTS output is louder than the original (by roughly 7-9 dB), so match the volume
before you compare.

Files are stored in `~/.local/share/acerultrax/`.

## Known issues

- **PH16-71: headset microphone on the 3.5 mm jack does not work on Linux kernels from before August 2026.**
  This is a kernel issue, not AcerUltraX (which never touches microphones); the fix is in Linux (commit
  `04a8e286bb2e`). On an older kernel, add it as a driver option and reboot:

  ```sh
  echo 'options snd_sof_intel_hda_generic hda_model=alc2xx-fixup-headset-mic' | sudo tee /etc/modprobe.d/acerultrax-headset-mic.conf
  ```

## Roadmap

1. ✅ Native DTS engine, bit-exact with Windows; importer for Acer's driver package
2. ✅ Per-app processing like Windows (each app's stream in its own channel layout)
3. ✅ Live system-wide audio through PipeWire, following the output you pick, headphones and Bluetooth
4. ✅ Background service, command-line control
5. ✅ A profile per output device (speakers, wired headphones, Bluetooth), autostart
6. ✅ The app: mode switching, loudness-matched comparison, devices, settings, top-bar icon, first-run setup
7. ✅ Headphone models (over 500, from Acer's and DTS's databases), recognised automatically over Bluetooth/USB
8. ✅ Profile maker: equaliser and DTS controls, with the measured response curve
9. ✅ AUR package; .deb/.rpm to follow
10. ✅ Windows "Spatial sound": DTS:X Ultra (speakers) and DTS Headphone:X, for 5.1/7.1 sound
11. The app icon

## Building and testing

See [docs/TESTING.md](docs/TESTING.md). The code is split into crates under `crates/`:

| Crate | What it does |
|---|---|
| `pe-loader` | Loads 64-bit Windows DLLs into a Linux process and provides the Windows functions they use |
| `dts-eagle` | Safe interface to the DTS engine; reads Acer's tuning configuration |
| `importer` | Detects the sound chip; sets up the DTS files from Acer's package |
| `dts-spatial` | Windows' spatial sound encoder (DTS:X Ultra / Headphone:X), loaded natively |
| `audio-graph` | The Windows signal chain: an engine per app, mix, mode engine, loudness match |
| `daemon` | `acerultrax-daemon`: PipeWire routing, device following, profiles, control socket |
| `cli` | `acerultrax-cli` |
| `gui` | `acerultrax`: the app and its top-bar icon |

## Legal

AcerUltraX is an independent project, not affiliated with or endorsed by Acer, DTS or Xperi. DTS and DTS:X are
trademarks of DTS, Inc.; Acer and Predator are trademarks of Acer Inc. AcerUltraX does not include or
redistribute their software: it downloads it from Acer, on your machine, for your own laptop.

## Licence

MIT, © 2026 Fawaz Al-Ghzawi. You may use, change and share AcerUltraX freely, including in your own projects;
keep the copyright notice in [LICENSE](LICENSE) so the original author is credited.
