{
  lib,
  rustPlatform,
  pkg-config,
  pipewire,
  gtk4,
  wrapGAppsHook4,
  withGui ? true,
}:

let
  cargoToml = builtins.fromTOML (builtins.readFile ../../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = "acerultrax" + lib.optionalString (!withGui) "-headless";
  inherit (cargoToml.workspace.package) version;

  src = lib.cleanSource ../../.;

  cargoLock = {
    lockFile = ../../Cargo.lock;
  };

  nativeBuildInputs = [
    pkg-config
    rustPlatform.bindgenHook
  ] ++ lib.optionals withGui [
    wrapGAppsHook4
  ];

  buildInputs = [
    pipewire
  ] ++ lib.optionals withGui [
    gtk4
  ];

  cargoBuildFlags = lib.optionals (!withGui) [
    "-p"
    "acerultrax-daemon"
    "-p"
    "acerultrax-cli"
  ];

  cargoTestFlags = lib.optionals (!withGui) [
    "-p"
    "acerultrax-daemon"
    "-p"
    "acerultrax-cli"
  ];

  postPatch = lib.optionalString withGui ''
    substituteInPlace crates/gui/src/tray.rs \
      --replace-fail "/usr/share/icons/hicolor/symbolic/apps/io.github.fawaz7.AcerUltraX-symbolic.svg" \
                     "$out/share/icons/hicolor/symbolic/apps/io.github.fawaz7.AcerUltraX-symbolic.svg"
  '';

  postInstall = ''
    install -Dm644 packaging/acerultrax.service $out/lib/systemd/user/acerultrax.service
    substituteInPlace $out/lib/systemd/user/acerultrax.service \
      --replace-fail "/usr/bin/acerultrax-daemon" "$out/bin/acerultrax-daemon"
  '' + lib.optionalString withGui ''
    install -Dm644 packaging/io.github.fawaz7.AcerUltraX.desktop $out/share/applications/io.github.fawaz7.AcerUltraX.desktop
    mkdir -p $out/share/icons
    cp -r data/icons/hicolor $out/share/icons/
  '';

  meta = with lib; {
    description = "DTS:X Ultra sound for Acer laptops on Linux";
    homepage = "https://github.com/fawaz7/AcerUltraX";
    license = licenses.mit;
    platforms = [ "x86_64-linux" ];
    mainProgram = if withGui then "acerultrax" else "acerultrax-cli";
  };
}
