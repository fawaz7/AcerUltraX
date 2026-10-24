# Changelog

## 0.1.2 (2026-10-24)

- Fixed: 5.1 and 7.1 sound (for example a movie in VLC) lost its centre channel, where most dialogue is, when DTS
  was off and in Compare. It is now mixed down to stereo exactly as Linux does without AcerUltraX, so Compare is
  fair on movies too ([#2](https://github.com/fawaz7/AcerUltraX/issues/2), reported by u/Natethegreat9999 on
  Reddit).
- Fixed: on laptops whose extra speakers (woofers or a rear pair) show up as a 2.1 or 4.0 output, those speakers
  stayed silent. They now get sound, as the sound card gives them on its own.
- Fixed: with DTS on, a 5.1/7.1 sound could click about 40 ms after it started
  ([#3](https://github.com/fawaz7/AcerUltraX/issues/3)).
- Fixed: apps that use PipeWire directly (mpv, some games) lost the first 20 ms of every sound.
- Compare is much more accurate ([#3](https://github.com/fawaz7/AcerUltraX/issues/3)): it allows for DTS's own
  delay, follows DTS's loudness control within about a second, and is right the moment it is switched on.
- The speakers now go to sleep when nothing plays, as without AcerUltraX, instead of staying powered all the time.
- Apps you have sent to another output, and apps marked not to be moved, are no longer taken over.

## 0.1.1 (2026-10-23)

- Fixed: the top-bar menu showed no profiles after logging in, until the Top bar icon switch in Settings was
  turned off and on again.

## 0.1.0 (2026-10-20)

First release: Acer's sound modes matching Windows, a profile per output, over 500 headphone models, spatial sound,
your own equaliser profiles, top-bar icon, one-click setup.
