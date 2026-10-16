# Read-only evaluation; does not activate a service or modify this machine.
let
  flake = builtins.getFlake (toString ../.);
  evaluated = flake.inputs.nixpkgs.lib.nixosSystem {
    system = builtins.currentSystem;
    modules = [
      flake.nixosModules.default
      ({ ... }: {
        system.stateVersion = "26.05";
        services.larpgym = {
          enable = true;
          package = flake.packages.${builtins.currentSystem}.default;
          tokenFile = "/run/secrets/gym-token";
        };
      })
    ];
  };
in evaluated.config.systemd.services.larpgym.serviceConfig
