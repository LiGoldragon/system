{
  description = "Portable OS and window-manager boundary for Persona.";

  inputs = {
    nixpkgs.url = "github:LiGoldragon/nixpkgs?ref=main";

    fenix.url = "github:nix-community/fenix";
    fenix.inputs.nixpkgs.follows = "nixpkgs";

    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      fenix,
      crane,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forSystems = function: nixpkgs.lib.genAttrs systems (system: function system);
      mkContext =
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          toolchain = fenix.packages.${system}.stable.withComponents [
            "cargo"
            "rustc"
            "rustfmt"
            "clippy"
            "rust-src"
          ];
          craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;
          schemaFilter =
            path: type:
            (type == "regular" || type == "directory") && (builtins.match ".*/schema(/.*)?" path != null);
          sourceFilter = path: type: (craneLib.filterCargoSources path type) || (schemaFilter path type);
          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = sourceFilter;
            name = "source";
          };
          commonArgs = {
            inherit src;
            strictDeps = true;
          };
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
          cargoTest =
            testTarget: testName:
            craneLib.cargoTest (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoTestExtraArgs = "--test ${testTarget} ${testName} -- --exact";
              }
            );
        in
        {
          inherit
            pkgs
            toolchain
            craneLib
            commonArgs
            cargoArtifacts
            cargoTest
            ;
        };
    in
    {
      packages = forSystems (
        system:
        let
          context = mkContext system;
        in
        {
          default = context.craneLib.buildPackage (
            context.commonArgs
            // {
              inherit (context) cargoArtifacts;
              pname = "system";
              meta.mainProgram = "system";
            }
          );
        }
      );

      checks = forSystems (
        system:
        let
          context = mkContext system;
        in
        {
          default = context.craneLib.cargoTest (
            context.commonArgs
            // {
              inherit (context) cargoArtifacts;
            }
          );
          system-daemon-applies-spawn-envelope-socket-mode = context.cargoTest "daemon" "daemon_binds_sockets_at_the_managed_mode";
          system-daemon-answers-status-readiness = context.cargoTest "daemon" "daemon_answers_status_readiness";
          system-daemon-answers-component-supervision-relation = context.cargoTest "daemon" "daemon_answers_component_supervision_relation";
          system-daemon-answers-meta-system-relation = context.cargoTest "daemon" "daemon_answers_meta_system_relation_with_typed_paused_reply";
          system-daemon-returns-typed-unimplemented = context.cargoTest "daemon" "daemon_returns_typed_unimplemented";
          system-cli-reaches-working-socket = context.cargoTest "component_cli" "system_cli_reaches_working_socket_and_prints_typed_reply";
          meta-system-cli-reaches-policy-socket = context.cargoTest "component_cli" "meta_system_cli_reaches_policy_socket_and_prints_typed_reply";
        }
      );

      apps = forSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/system";
        };
        daemon = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/system-daemon";
        };
        focus = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/system-focus";
        };
        meta = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/meta-system";
        };
      });

      devShells = forSystems (
        system:
        let
          context = mkContext system;
        in
        {
          default = context.pkgs.mkShell {
            packages = [
              context.pkgs.jujutsu
              context.pkgs.pkg-config
              context.toolchain
            ];
          };
        }
      );

      formatter = forSystems (
        system:
        let
          context = mkContext system;
        in
        context.pkgs.nixfmt
      );
    };
}
