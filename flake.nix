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
        "build.rs"
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
      clangDeps = with pkgs; [
        pkg-config
        llvmPackages.clang
        llvmPackages.libclang
      ];
      libclangPath = "${pkgs.llvmPackages.libclang.lib}/lib";

      evalModuleConfig =
        cfg:
        (nixpkgs.lib.nixosSystem {
          inherit system;
          modules = [
            self.nixosModules.default
            { services.ao3instance = cfg; }
          ];
        }).config;

      ao3instance = pkgs.rustPlatform.buildRustPackage {
        pname = "ao3instance";
        version = (pkgs.lib.importTOML ./Cargo.toml).package.version;

        inherit src;
        cargoLock.lockFile = ./Cargo.lock;

        nativeBuildInputs = clangDeps;
        buildInputs = nativeDeps;

        LIBCLANG_PATH = libclangPath;

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
        inherit ao3instance;
      };

      nixosModules.default = import ./nix/module.nix { package = ao3instance; };

      checks.${system} = {
        ao3instance-module-open-firewall =
          let
            exposed = evalModuleConfig {
              enable = true;
              dataDir = "/srv/ao3instance-test";
              port = 9123;
              openFirewall = true;
            };
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
            ! grep -q "^ENV=AO3I_WRITE_ENABLED=" ${values}
            touch $out
          '';

        ao3instance-module-writes-enabled =
          let
            enabled = evalModuleConfig {
              enable = true;
              writesEnabled = true;
            };
            values = pkgs.writeText "ao3instance-module-writes-enabled.txt" ''
              ${builtins.concatStringsSep "\n" (map (value: "ENV=${value}") enabled.systemd.services.ao3instance.serviceConfig.Environment)}
            '';
          in
          pkgs.runCommand "check-ao3instance-module-writes-enabled" { } ''
            grep -qx "ENV=AO3I_WRITE_ENABLED=true" ${values}
            touch $out
          '';

        ao3instance-module-flaresolverr-port =
          let
            configured = evalModuleConfig {
              enable = true;
              flaresolverrPort = 8192;
            };
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
              writesEnabled = true;
            };
          };
          testScript = ''
            machine.wait_for_unit("ao3instance.service")
            machine.wait_for_open_port(9123)
            machine.succeed("curl -fsS http://127.0.0.1:9123/health | grep -qx ok")
            machine.succeed("test -s /var/lib/ao3instance-test/ao3i.db")
            machine.succeed("test -s /var/lib/ao3instance-test/key")
            machine.succeed(
              "systemctl show ao3instance.service -p Environment | grep -q 'AO3I_WRITE_ENABLED=true'"
            )
          '';
        };
      };

      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs =
          nativeDeps
          ++ clangDeps
          ++ (with pkgs; [
            rustc
            cargo
            clippy
            rustfmt
            rust-analyzer
            sqlx-cli
            cargo-deny
          ]);

        LIBCLANG_PATH = libclangPath;
        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
      };
    };
}
