{ lib, rustPlatform }:

let
  cargoToml = builtins.fromTOML (builtins.readFile ../../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = "acerultrax-cli";
  inherit (cargoToml.workspace.package) version;

  src = lib.cleanSource ../../.;

  cargoLock = {
    lockFile = ../../Cargo.lock;
  };

  cargoBuildFlags = [ "-p" "acerultrax-cli" ];
  cargoTestFlags = [ "-p" "acerultrax-cli" ];

  meta = with lib; {
    description = "Command-line tool for AcerUltraX (DTS:X Ultra on Acer laptops)";
    homepage = "https://github.com/fawaz7/AcerUltraX";
    license = licenses.mit;
    platforms = [ "x86_64-linux" ];
    mainProgram = "acerultrax-cli";
  };
}
