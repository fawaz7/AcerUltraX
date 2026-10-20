# AcerUltraX user guide

AcerUltraX gives your Acer laptop the DTS:X Ultra sound it has on Windows: the same modes (Music, Movie, Game,
Voice, Custom Audio), tuned by Acer for your laptop's speakers and for headphones.

## Install

**Arch Linux, CachyOS, Manjaro, EndeavourOS** (from the AUR):

```sh
yay -S acerultrax-git
```

Other distributions: build from source (see the README). You need PipeWire with WirePlumber, which is the
default on current Fedora, Ubuntu, Debian, openSUSE and Arch.

## First start

Open **AcerUltraX** from your applications. The first time, it walks you through setup:

1. It shows your laptop and whether it is supported. The Predator Helios 16 (PH16-71) is verified against
   Windows; other Acer laptops with DTS:X Ultra should work too, but have not been checked yet. If you have one,
   please [tell us how it works](https://github.com/fawaz7/AcerUltraX/issues).
2. It opens Acer's support page. Search for your laptop and download:
   - **Driver → Audio: "XPERI DTS"** (the DTS engine and your laptop's tuning; required);
   - **Application: "XPERI DTS Utility"** (headphone models; take both versions if two are listed).
   Do not unzip them.
3. Click **Set up AcerUltraX**. It finds the downloads in your Downloads folder, sets everything up in a few
   seconds and starts DTS. From now on it starts with your session.

AcerUltraX does not include any Acer or DTS files: it uses the ones you download for your own laptop.

## Everyday use

- **Volume:** use your desktop's volume control as usual. It applies after DTS, as on Windows, with the same
  feel (half volume is about as loud as half volume on Windows).
- **Output device:** pick it in your desktop's sound settings as usual. AcerUltraX follows: plug in headphones
  or connect Bluetooth and it switches by itself.
- **Sound mode:** on the **Sound** page, or from the top-bar icon (turn it on under Settings; on GNOME it needs
  the AppIndicator extension).
- **On/off:** the switch at the top of the Sound page turns DTS off for everything (plain sound).
- **Compare:** plays plain sound at the same loudness as DTS, so you hear only what DTS changes. DTS is louder,
  and louder always sounds "better", so this is the fair way to compare.

## Spatial sound

Like Windows' "Spatial sound: DTS:X Ultra", AcerUltraX can place 5.1 and 7.1 sound (games, films) around you:
DTS:X Ultra on the laptop speakers, DTS Headphone:X on headphones. Turn it on per device with the **Spatial
sound** switch on the Sound page (it appears once the "XPERI DTS Utility" 2023 download is set up). Stereo
sound is not changed by it, as on Windows. Settings → "Spatial sound: save power" uses DTS's lighter presets
(Windows' "Best power").

## Devices

Each output keeps its own mode:

- **Laptop speakers** and **Headphones (3.5 mm jack)** are always listed, so you can set the headphone mode
  before plugging in.
- **Bluetooth** devices are remembered once connected, each with its own mode. Headphones get DTS's headphone
  tuning; Bluetooth speakers get plain sound unless you choose a mode.
- **Headphone model:** like Windows' DTS app, pick your headphones from over 500 models so DTS can tune for
  them. Bluetooth and USB headphones are recognised automatically. (HyperX Cloud II on the 3.5 mm jack: choose
  **HyperX Cloud Core**, the same headset without the USB sound card.)
- HDMI and other outputs get plain sound unless you choose a mode.

## Your own profiles

On the **Profiles** page, **+ New profile** starts a profile from any mode. Adjust:

- the **10-band equaliser** (31 Hz to 16 kHz);
- **Bass** (DTS TruBass), **Dialogue clarity**, **Surround** with its room type, **Loudness control**.

The curve at the top is measured: AcerUltraX plays a test signal through DTS (silently, inside the app) and shows
how each frequency comes out, so it is exactly what you will hear, for the output in use. Changes are saved and
heard right away. Your profiles appear on the Sound page and in the top-bar menu.

"Off" is DTS's own neutral mode (on the laptop speakers it keeps Acer's speaker protection, as on Windows);
"Plain" is no processing at all.

## Command line

```sh
acerultrax-cli status            # what is playing where
acerultrax-cli mode movie        # music, movie, voice, game1, game2, game3, "custom audio", off, plain
acerultrax-cli devices           # outputs and their modes
acerultrax-cli headphones hyperx # search headphone models
acerultrax-cli spatial on        # spatial sound for 5.1/7.1 on the current output
acerultrax-cli power off         # plain sound everywhere (power on to return)
acerultrax-cli autostart off     # stop starting with the session
```

## Troubleshooting

- **No sound after setup:** check that the output you want is selected in your desktop's sound settings, then
  `acerultrax-cli status`. `systemctl --user restart acerultrax` restarts the service.
- **Turn it off completely:** `acerultrax-cli autostart off`. Your sound returns to normal immediately.
- **PH16-71: the headset microphone on the 3.5 mm jack does not work** (Linux kernels before August 2026): see
  "Known issues" in the README.
- Logs: `journalctl --user -u acerultrax`.
