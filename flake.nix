{
  description = "AO3Instance — an offline-capable AO3 reader and write client";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };

      # Whitelist what the build reads. A denylist leaked the reference clone
      # (otwarchive/) and local runtime state into the store copy.
      #
      # Note: the filter must admit everything *under* a tree, not just the tree
      # directory itself. The first version used `builtins.elem rel trees`, which
      # passed `src/` but rejected `src/lib.rs` — the build failed with "no
      # targets specified". (MUSIKA's flake has the same shape; do not copy it.)
      allowTrees = [
        "src"
        "tests"
        "migrations"
        "scripts"
      ];
      allowFiles = [
        "Cargo.toml"
        "Cargo.lock"
        "clippy.toml"
        "README.md"
        "flake.nix"
      ];
      underTree =
        rel:
        builtins.any (tree: rel == tree || pkgs.lib.hasPrefix (tree + "/") rel) allowTrees;
      src = pkgs.lib.cleanSourceWith {
        src = ./.;
        filter =
          path: type:
          let
            rel = pkgs.lib.removePrefix (toString ./. + "/") (toString path);
          in
          (type == "directory" && underTree rel)
          || (type == "regular" && (builtins.elem rel allowFiles || underTree rel));
      };

      nixosTest = import (nixpkgs.outPath + "/nixos/lib/testing-python.nix") {
        inherit system;
      };

      # Runtime/build inputs shared by the package and the dev shell.
      # openssl backs both reqwest's native TLS and our AES-256-GCM secret store.
      nativeDeps = with pkgs; [
        pkg-config
        openssl
        sqlite
      ];

      ao3instance = pkgs.rustPlatform.buildRustPackage {
        pname = "ao3instance";
        version = (pkgs.lib.importTOML ./Cargo.toml).package.version;

        inherit src;
        cargoLock.lockFile = ./Cargo.lock;

        nativeBuildInputs = with pkgs; [
          pkg-config
          llvmPackages.clang
          llvmPackages.libclang
        ];
        buildInputs = nativeDeps;

        LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

        # Tests run in CI/dev shell, not in the package build (same split as MUSIKA).
        doCheck = false;

        meta = {
          description = "Offline-capable AO3 reader and write client";
        };
      };
    in
    {
      packages.${system} = {
        default = ao3instance;
        ao3instance = ao3instance;
      };

      nixosModules.default = import ./nix/module.nix { package = ao3instance; };

      checks.${system} = {
        ao3instance-module-open-firewall =
        let
          exposed = (nixpkgs.lib.nixosSystem {
            inherit system;
            modules = [
              self.nixosModules.default
              {
                services.ao3instance = {
                  enable = true;
                  dataDir = "/srv/ao3instance-test";
                  port = 9123;
                  openFirewall = true;
                };
              }
            ];
          }).config;
          values = pkgs.writeText "ao3instance-module-open-firewall.txt" ''
            DATA_DIR=${toString exposed.services.ao3instance.dataDir}
            PORTS=${builtins.concatStringsSep "," (map toString exposed.networking.firewall.allowedTCPPorts)}
            ${builtins.concatStringsSep "\n" (map (value: "ENV=${value}") exposed.systemd.services.ao3instance.serviceConfig.Environment)}
          '';
        in
        pkgs.runCommand "check-ao3instance-module-open-firewall" { } ''
          grep -qx "DATA_DIR=/srv/ao3instance-test" ${values}
          grep -qx "PORTS=9123" ${values}
          grep -q "ENV=AO3I_BIND=0.0.0.0" ${values}
          ! grep -q "^ENV=AO3I_FLARESOLVERR_URL=" ${values}
          touch $out
        '';

        ao3instance-module-flaresolverr-port =
        let
          configured = (nixpkgs.lib.nixosSystem {
            inherit system;
            modules = [
              self.nixosModules.default
              {
                services.ao3instance = {
                  enable = true;
                  flaresolverrPort = 8192;
                };
              }
            ];
          }).config;
          values = pkgs.writeText "ao3instance-module-flaresolverr-port.txt" ''
            ${builtins.concatStringsSep "\n" (map (value: "ENV=${value}") configured.systemd.services.ao3instance.serviceConfig.Environment)}
          '';
        in
        pkgs.runCommand "check-ao3instance-module-flaresolverr-port" { } ''
          grep -qx "ENV=AO3I_FLARESOLVERR_URL=http://127.0.0.1:8192" ${values}
          touch $out
        '';

        ao3instance-module = nixosTest.runTest {
        name = "ao3instance-module";
        nodes.machine = {
          imports = [ self.nixosModules.default ];
          services.ao3instance = {
            enable = true;
            dataDir = "/var/lib/ao3instance-test";
            port = 9123;
          };
        };
        testScript = ''
          machine.wait_for_unit("ao3instance.service")
          machine.wait_for_open_port(9123)
          machine.succeed("curl -fsS http://127.0.0.1:9123/health | grep -qx ok")
          machine.succeed("test -s /var/lib/ao3instance-test/ao3i.db")
          machine.succeed("test -s /var/lib/ao3instance-test/key")
        '';
      };

      };

      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [
          rustc
          cargo
          clippy
          rustfmt
          rust-analyzer
          pkg-config
          openssl
          sqlite
          sqlx-cli
          llvmPackages.clang
          llvmPackages.libclang
          cargo-deny
        ];

        LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
      };
    };
}
