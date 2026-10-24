{
  description = "AcerUltraX: DTS:X Ultra sound for Acer laptops on Linux";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ self.overlays.default ];
      };
    in
    {
      overlays.default = final: prev: {
        acerultrax = final.callPackage ./packaging/nix { };
        acerultrax-headless = final.acerultrax.override { withGui = false; };
      };

      packages.${system} = {
        acerultrax = pkgs.acerultrax;
        acerultrax-headless = pkgs.acerultrax-headless;
        default = pkgs.acerultrax;
      };

      apps.${system} = {
        acerultrax = {
          type = "app";
          program = "${self.packages.${system}.acerultrax}/bin/acerultrax";
          meta.description = "GTK4 graphical interface for AcerUltraX";
        };
        acerultrax-cli = {
          type = "app";
          program = "${self.packages.${system}.acerultrax}/bin/acerultrax-cli";
          meta.description = "Command-line tool for AcerUltraX";
        };
        acerultrax-daemon = {
          type = "app";
          program = "${self.packages.${system}.acerultrax}/bin/acerultrax-daemon";
          meta.description = "Background PipeWire service for AcerUltraX";
        };
        default = self.apps.${system}.acerultrax;
      };

      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [
          cargo
          rustc
          rustPlatform.bindgenHook
          pkg-config
          wrapGAppsHook4
          clang
          lld
          llvm
        ];

        buildInputs = with pkgs; [
          pipewire
          gtk4
          glib
        ];
      };

      homeManagerModules = {
        acerultrax = import ./packaging/nix/home-manager-module.nix;
        default = self.homeManagerModules.acerultrax;
      };
    };
}
