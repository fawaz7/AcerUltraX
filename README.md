# AcerUltraX

Your Acer laptop's Windows sound, on Linux.

Acer gaming laptops ship with **DTS:X Ultra** on Windows: per-mode tunings (Music, Movie, Voice, Game, Custom
Audio) made for the laptop's own speakers and for headphones. On Linux the same speakers sound flat and thin.
AcerUltraX runs Acer's original DTS engine natively on Linux, without Wine, and processes your audio the way
Windows does, sample for sample.

> **Status: early development.** The DTS engine runs natively and matches Windows bit for bit in offline tests.
> Live system-wide audio, the app window, profiles and per-device switching are being built (see
> [Roadmap](#roadmap)). There is nothing to install for everyday use yet.

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

- AcerUltraX contains **no DTS or Acer files**. You provide Acer's own driver package for your laptop, and
  AcerUltraX loads the DTS engine from it with a small built-in loader for Windows libraries.
- It applies the same factory settings Windows does, from Acer's tuning files for your laptop.
- It contains no licence checks. The DTS engine checks none on Linux either: on Windows, licensing lives in
  other parts of the DTS software, which AcerUltraX does not use.

Requirements: a 64-bit x86 (x86_64) Acer laptop, Linux with PipeWire (for live audio, once available).

## Try it (developers)

```sh
cargo build --release
./target/release/acerultrax-cli detect          # shows your sound chip and whether it is supported
```

Get the DTS driver for your laptop from Acer's support site (Drivers → Audio, "XPERI DTS" or "DTS Audio"),
unzip it, then:

```sh
./target/release/acerultrax-cli import ~/Downloads/XPERI_DTS_folder
./target/release/acerultrax-cli status
./target/release/acerultrax-cli render song.wav song-dts.wav --mode movie --output-device speakers
```

`render` processes a WAV file (mono, stereo, 5.1 or 7.1) offline, so you can listen to DTS without changing
anything on your system. DTS output is louder than the original (by roughly 7-9 dB), so match the volume
before you compare.

Files are stored in `~/.local/share/acerultrax/`.

## Roadmap

1. ✅ Native DTS engine, bit-exact with Windows; importer for Acer's driver package
2. Per-app processing like Windows (each app's stream in its own channel layout)
3. Live system-wide audio through PipeWire, following speakers and headphones as you plug them
4. Background service, command-line control
5. Profiles, automatic profiles per output (speakers, wired headphones, Bluetooth), autostart
6. The app: mode switching, quick loudness-matched comparison, devices, settings
7. Windows "Spatial sound: DTS:X Ultra"
8. Profile maker: DTS controls and an equaliser with a live response curve
9. Packages (AUR first, then .deb/.rpm)

## Building and testing

See [docs/TESTING.md](docs/TESTING.md). The code is split into crates under `crates/`:

| Crate | What it does |
|---|---|
| `pe-loader` | Loads 64-bit Windows DLLs into a Linux process and provides the Windows functions they use |
| `dts-eagle` | Safe interface to the DTS engine; reads Acer's tuning configuration |
| `importer` | Detects the sound chip; sets up the DTS files from Acer's package |
| `cli` | `acerultrax-cli` |

## Legal

AcerUltraX is an independent project, not affiliated with or endorsed by Acer, DTS or Xperi. DTS and DTS:X are
trademarks of DTS, Inc.; Acer and Predator are trademarks of Acer Inc. AcerUltraX does not include or
redistribute their software: it uses the copy you obtain from Acer for your own laptop.

## Licence

MIT, © 2026 Fawaz Al-Ghzawi. You may use, change and share AcerUltraX freely, including in your own projects;
keep the copyright notice in [LICENSE](LICENSE) so the original author is credited.
