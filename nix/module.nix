{ package }:
{ config, lib, ... }:
let
  cfg = config.services.ao3instance;
in
{
  options.services.ao3instance = {
    enable = lib.mkEnableOption "AO3Instance";

    dataDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/ao3instance";
      description = ''
        Directory where AO3Instance writes its SQLite cache and AES key file.
        The service stores the database at <dataDir>/ao3i.db and the key at
        <dataDir>/key.
      '';
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 8099;
      description = ''
        TCP port the AO3Instance web UI listens on. It is loopback-only unless
        services.ao3instance.openFirewall is enabled.
      '';
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Open services.ao3instance.port in the NixOS firewall and bind the
        service to all interfaces. The app holds an AO3 session, so enable this
        only on trusted networks or put it behind a reverse proxy.
      '';
    };

    writesEnabled = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Enable AO3 write actions (kudos, comments and replies, subscriptions,
        posting and editing works and chapters, bookmark management). Off by
        default: a client that can write against the archive is an explicit
        operator decision.

        Writes need a usable AO3 session imported on the /session page (both
        the _otwarchive_session and user_credentials cookies). With this option
        on but no usable session, the UI explains that the session is unusable
        instead of pretending the reader is signed out.
      '';
    };

    flaresolverrPort = lib.mkOption {
      type = lib.types.nullOr lib.types.port;
      default = null;
      example = 8191;
      description = ''
        Loopback TCP port of a host-provided FlareSolverr service. When set,
        passes AO3I_FLARESOLVERR_URL=http://127.0.0.1:<port> to AO3Instance.
        When null, AO3Instance uses its built-in default
        (http://127.0.0.1:8191).
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = lib.hasPrefix "/" (toString cfg.dataDir);
        message = "services.ao3instance.dataDir must be an absolute path";
      }
      {
        assertion = cfg.flaresolverrPort == null || cfg.flaresolverrPort > 0;
        message = "services.ao3instance.flaresolverrPort must be a non-zero port";
      }
    ];

    users.groups.ao3instance = { };
    users.users.ao3instance = {
      isSystemUser = true;
      group = "ao3instance";
      home = toString cfg.dataDir;
      createHome = false;
    };

    systemd.tmpfiles.rules = [
      "d ${toString cfg.dataDir} 0700 ao3instance ao3instance -"
    ];

    networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ cfg.port ];

    systemd.services.ao3instance = {
      description = "AO3Instance";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [
        "network-online.target"
        "flaresolverr.service"
      ];

      serviceConfig = {
        Type = "simple";
        ExecStart = "${package}/bin/ao3i-server";
        User = "ao3instance";
        Group = "ao3instance";
        WorkingDirectory = toString cfg.dataDir;
        Environment =
          [
            "AO3I_DATA_DIR=${toString cfg.dataDir}"
            "AO3I_PORT=${toString cfg.port}"
          ]
          ++ lib.optional cfg.writesEnabled "AO3I_WRITE_ENABLED=true"
          ++ lib.optional cfg.openFirewall "AO3I_BIND=0.0.0.0"
          ++ lib.optional (
            cfg.flaresolverrPort != null
          ) "AO3I_FLARESOLVERR_URL=http://127.0.0.1:${toString cfg.flaresolverrPort}";
        Restart = "on-failure";
        RestartSec = 5;
        UMask = "0077";

        # The process only needs to read its binary/assets and write its data
        # directory. By default it binds loopback; openFirewall widens the bind
        # and opens the port explicitly.
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ (toString cfg.dataDir) ];
        RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" ];
      };
    };
  };
}
