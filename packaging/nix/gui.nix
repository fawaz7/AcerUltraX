{
  lib,
  rustPlatform,
  pkg-config,
  wrapGAppsHook4,
  gtk4,
}:

let
  cargoToml = builtins.fromTOML (builtins.readFile ../../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = "acerultrax-gui";
  inherit (cargoToml.workspace.package) version;

  src = lib.cleanSource ../../.;

  cargoLock = {
    lockFile = ../../Cargo.lock;
  };

  nativeBuildInputs = [
    pkg-config
    wrapGAppsHook4
  ];

  buildInputs = [
    gtk4
  ];

  cargoBuildFlags = [ "-p" "acerultrax" ];
  cargoTestFlags = [ "-p" "acerultrax" ];

  postPatch = ''
    substituteInPlace crates/gui/src/tray.rs \
      --replace-fail "/usr/share/icons/hicolor/symbolic/apps/io.github.fawaz7.AcerUltraX-symbolic.svg" \
                     "$out/share/icons/hicolor/symbolic/apps/io.github.fawaz7.AcerUltraX-symbolic.svg"
  '';

  postInstall = ''
    install -Dm644 packaging/io.github.fawaz7.AcerUltraX.desktop $out/share/applications/io.github.fawaz7.AcerUltraX.desktop
    mkdir -p $out/share/icons
    cp -r data/icons/hicolor $out/share/icons/
  '';

  meta = with lib; {
    description = "GTK4 graphical interface for AcerUltraX (DTS:X Ultra on Acer laptops)";
    homepage = "https://github.com/fawaz7/AcerUltraX";
    license = licenses.mit;
    platforms = [ "x86_64-linux" ];
    mainProgram = "acerultrax";
  };
}
