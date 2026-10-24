{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.acerultrax;

  defaultDaemon = pkgs.acerultrax-daemon or (pkgs.callPackage ./daemon.nix { });
  defaultCli = pkgs.acerultrax-cli or (pkgs.callPackage ./cli.nix { });
  defaultGui = pkgs.acerultrax-gui or (pkgs.callPackage ./gui.nix { });
in
{
  options.services.acerultrax = {
    enable = lib.mkEnableOption "AcerUltraX DTS:X Ultra sound service";

    package = lib.mkOption {
      type = lib.types.package;
      default = defaultDaemon;
      defaultText = lib.literalExpression "pkgs.acerultrax-daemon";
      description = "The AcerUltraX daemon package to run as a systemd user service.";
    };

    cli = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether to install the AcerUltraX command-line interface.";
      };
      package = lib.mkOption {
        type = lib.types.package;
        default = defaultCli;
        defaultText = lib.literalExpression "pkgs.acerultrax-cli";
        description = "The AcerUltraX CLI package to install.";
      };
    };

    gui = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether to install the AcerUltraX GTK4 graphical interface and desktop entry.";
      };
      package = lib.mkOption {
        type = lib.types.package;
        default = defaultGui;
        defaultText = lib.literalExpression "pkgs.acerultrax-gui";
        description = "The AcerUltraX GUI package to install.";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages =
      lib.optional cfg.cli.enable cfg.cli.package
      ++ lib.optional cfg.gui.enable cfg.gui.package;

    systemd.user.services.acerultrax = {
      Unit = {
        Description = "AcerUltraX: DTS:X Ultra sound for Acer laptops";
        After = [
          "pipewire.service"
          "wireplumber.service"
          "pipewire-pulse.service"
        ];
        Wants = [ "wireplumber.service" ];
      };

      Service = {
        ExecStart = "${cfg.package}/bin/acerultrax-daemon";
        Restart = "on-failure";
        RestartSec = 2;
      };

      Install = {
        WantedBy = [ "default.target" ];
      };
    };
  };
}
