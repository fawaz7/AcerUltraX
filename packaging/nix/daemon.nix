{
  lib,
  rustPlatform,
  pkg-config,
  pipewire,
}:

let
  cargoToml = builtins.fromTOML (builtins.readFile ../../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = "acerultrax-daemon";
  inherit (cargoToml.workspace.package) version;

  src = lib.cleanSource ../../.;

  cargoLock = {
    lockFile = ../../Cargo.lock;
  };

  nativeBuildInputs = [
    pkg-config
    rustPlatform.bindgenHook
  ];

  buildInputs = [
    pipewire
  ];

  cargoBuildFlags = [ "-p" "acerultrax-daemon" ];
  cargoTestFlags = [ "-p" "acerultrax-daemon" ];

  postInstall = ''
    install -Dm644 packaging/acerultrax.service $out/lib/systemd/user/acerultrax.service
    substituteInPlace $out/lib/systemd/user/acerultrax.service \
      --replace-fail "/usr/bin/acerultrax-daemon" "$out/bin/acerultrax-daemon"
  '';

  meta = with lib; {
    description = "Background PipeWire service for AcerUltraX (DTS:X Ultra on Acer laptops)";
    homepage = "https://github.com/fawaz7/AcerUltraX";
    license = licenses.mit;
    platforms = [ "x86_64-linux" ];
    mainProgram = "acerultrax-daemon";
  };
}
