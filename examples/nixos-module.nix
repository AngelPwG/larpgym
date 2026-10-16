{ config, lib, pkgs, ... }:
let
  cfg = config.services.larpgym;
in {
  options.services.larpgym = {
    enable = lib.mkEnableOption "backend personal LarpGym";
    package = lib.mkOption { type = lib.types.package; description = "Paquete LarpGym compilado para la arquitectura de esta laptop"; };
    bind = lib.mkOption { type = lib.types.str; default = "127.0.0.1:8743"; description = "Dirección:puerto elegido por el administrador"; };
    tokenFile = lib.mkOption { type = lib.types.str; description = "Ruta ABSOLUTA fuera del Nix store a un secreto root:root 0600"; };
  };
  config = lib.mkIf cfg.enable {
    assertions = [{ assertion = lib.hasPrefix "/" cfg.tokenFile && !(lib.hasPrefix "/nix/store/" cfg.tokenFile); message = "larpgym.tokenFile debe ser una ruta absoluta fuera del Nix store"; }];
    users.groups.larpgym = {};
    users.users.larpgym = { isSystemUser = true; group = "larpgym"; home = "/var/lib/larpgym"; };
    systemd.services.larpgym = {
      description = "LarpGym API v1";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      environment = {
        GYM_BIND = cfg.bind;
        GYM_SERVER_DB = "/var/lib/larpgym/server.sqlite";
        GYM_TOKEN_FILE = "%d/gym-token";
      };
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/gym-server";
        User = "larpgym";
        Group = "larpgym";
        StateDirectory = "larpgym";
        StateDirectoryMode = "0700";
        LoadCredential = "gym-token:${cfg.tokenFile}";
        Restart = "on-failure";
        RestartSec = 5;
        UMask = "0077";
        NoNewPrivileges = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ProtectKernelTunables = true;
        ProtectControlGroups = true;
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_UNIX" ];
      };
    };
  };
}
