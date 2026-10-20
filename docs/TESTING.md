# Testing

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Everything runs in public CI except the tests that need files this repository does not contain. Those skip
themselves (and say so) unless you point them at local copies:

| Variable | Contents | Used by |
|---|---|---|
| `ACERULTRAX_VENDOR_DIR` | a folder with `dtstech64.dll`, `dts_apo4_oem_config_<ID>.xml` and the tuning files, e.g. `~/.local/share/acerultrax/dts` after an import | `dts-eagle` reference tests |
| `ACERULTRAX_GOLDEN_DIR` | reference recordings: engine input and output captured inside the Windows DTS software, with `index.json` | `dts-eagle` reference tests |

```sh
ACERULTRAX_VENDOR_DIR=~/.local/share/acerultrax/dts ACERULTRAX_GOLDEN_DIR=/path/to/golden \
  cargo test -p dts-eagle --test golden -- --nocapture
```

What the reference tests check:

- **from blobs**: engines loaded with the recorded engine state reproduce the recorded output exactly (every
  mode, speakers and headphones, 44.1 and 48 kHz, and a 7.1 stream).
- **from controls**: engines configured setting by setting from Acer's configuration file reproduce it exactly.
  The recordings were made in a state where the engine's file-based tuning (speaker correction and per-mode EQ
  files) had not loaded, so this test leaves those settings out to match.
- **full tuning**: with every setting, including the tuning files, an engine configured from settings and one
  loaded from its saved state give identical output, and the tuning files change the sound where a mode uses
  them.
- **concurrent**: engines on several threads at once stay exact.

The loader's own tests build a small Windows DLL from `crates/pe-loader/tests/testdll/testdll.c`. They need
clang, lld (`lld-link`) and llvm (`llvm-dlltool`), and skip without them.

Notes:
- The DTS engine needs a thread stack of at least 1 MiB; tests run engine work on 8 MiB threads.
- Compare engines only on identical input: the engine's loudness control adapts slowly, so output depends on
  what came before.
- Never compare tuning blobs byte for byte: a few bytes in them are uninitialised memory and differ from run to
  run. Compare the audio instead.
