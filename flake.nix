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
        acerultrax-cli = final.callPackage ./packaging/nix/cli.nix { };
        acerultrax-daemon = final.callPackage ./packaging/nix/daemon.nix { };
        acerultrax-gui = final.callPackage ./packaging/nix/gui.nix { };
      };

      packages.${system} = {
        acerultrax-cli = pkgs.acerultrax-cli;
        acerultrax-daemon = pkgs.acerultrax-daemon;
        acerultrax-gui = pkgs.acerultrax-gui;
        default = pkgs.acerultrax-gui;
      };

      apps.${system} = {
        acerultrax-cli = {
          type = "app";
          program = "${self.packages.${system}.acerultrax-cli}/bin/acerultrax-cli";
          meta.description = "Command-line tool for AcerUltraX";
        };
        acerultrax-daemon = {
          type = "app";
          program = "${self.packages.${system}.acerultrax-daemon}/bin/acerultrax-daemon";
          meta.description = "Background PipeWire service for AcerUltraX";
        };
        acerultrax = {
          type = "app";
          program = "${self.packages.${system}.acerultrax-gui}/bin/acerultrax";
          meta.description = "GTK4 graphical interface for AcerUltraX";
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
