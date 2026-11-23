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
      description = "Loopback TCP port the AO3Instance web UI listens on.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = lib.hasPrefix "/" (toString cfg.dataDir);
        message = "services.ao3instance.dataDir must be an absolute path";
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

    systemd.services.ao3instance = {
      description = "AO3Instance";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = "${package}/bin/ao3i-server";
        User = "ao3instance";
        Group = "ao3instance";
        WorkingDirectory = toString cfg.dataDir;
        Environment = [
          "AO3I_DATA_DIR=${toString cfg.dataDir}"
          "AO3I_PORT=${toString cfg.port}"
        ];
        Restart = "on-failure";
        RestartSec = 5;
        UMask = "0077";

        # The process only needs to read its binary/assets and write its data
        # directory. It binds loopback, so the module does not open a firewall.
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
