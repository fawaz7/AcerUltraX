{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.acerultrax;

  defaultPackage = pkgs.acerultrax or (pkgs.callPackage ./. { });
  effectivePackage =
    if cfg.gui.enable then
      cfg.package
    else
      cfg.package.override { withGui = false; };
in
{
  options.services.acerultrax = {
    enable = lib.mkEnableOption "AcerUltraX DTS:X Ultra sound service";

    package = lib.mkOption {
      type = lib.types.package;
      default = defaultPackage;
      defaultText = lib.literalExpression "pkgs.acerultrax";
      description = "The AcerUltraX package to use.";
    };

    gui = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Whether to build and install the GTK4 graphical interface and desktop entry. If disabled, a headless build with only the daemon and CLI is installed.";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ effectivePackage ];

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
        ExecStart = "${effectivePackage}/bin/acerultrax-daemon";
        Restart = "on-failure";
        RestartSec = 2;
      };

      Install = {
        WantedBy = [ "default.target" ];
      };
    };
  };
}
