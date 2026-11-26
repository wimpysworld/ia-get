{
  description = "ia-get";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    tailor.url = "github:wimpysworld/tailor/latest";
  };

  outputs =
    {
      self,
      nixpkgs,
      tailor,
      ...
    }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-darwin"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          package = import ./nix/package.nix { inherit pkgs self; };
        in
        # Expose the project package under its pname and as the default package,
        # only on systems that its meta.platforms allows.
        pkgs.lib.optionalAttrs
          (
            builtins.pathExists ./nix/package.nix && pkgs.lib.meta.availableOn pkgs.stdenv.hostPlatform package
          )
          {
            ${package.pname} = package;
            default = package;
          }
      );
      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          # Install static Fira Code TTF faces (Regular, Bold, etc.) rather than the
          # default variable font (FiraCode-VF.ttf, whose NameIDFamily is "Fira Code Light"),
          # so ntrecord resolves `Set FontFamily "Fira Code"` with both regular and bold faces.
          firaCode = pkgs.fira-code.overrideAttrs { preInstall = "cd ttf"; };
          ntrecordBase = import ./nix/ntrecord.nix { inherit pkgs; };
          # ntrecord resolves named primary fonts via XDG_DATA_HOME/fonts (NTRECORD_FONT_PATH
          # only configures fallback font roots), so scope XDG_DATA_HOME to the ntrecord wrapper
          # without overriding the user's shell XDG_DATA_HOME.
          ntrecord = pkgs.symlinkJoin {
            name = "ntrecord-${ntrecordBase.version}";
            paths = [ ntrecordBase ];
            nativeBuildInputs = [ pkgs.makeWrapper ];
            postBuild = ''
              wrapProgram $out/bin/ntrecord \
                --set XDG_DATA_HOME "${firaCode}/share" \
                --set-default NTRECORD_FONT_PATH "${firaCode}/share/fonts/truetype"
            '';
          };
          # Keep the development shell available when Tailor has no package for this system.
          tailorPkgs = tailor.packages.${system} or { };
        in
        {
          default = pkgs.mkShell {
            NTRECORD_FONT_PATH = "${firaCode}/share/fonts/truetype";
            shellHook = import ./nix/hooks.nix { inherit pkgs; };
            packages =
              with pkgs;
              [
                actionlint
                cargo-edit
                cargo-watch
                cargo-zigbuild
                ffmpeg-headless
                firaCode
                gh
                goreleaser
                just
                ntrecord
                rust-analyzer
                zig
              ]
              ++ (if tailorPkgs ? tailor then [ tailorPkgs.tailor ] else [ ])
              ++ import ./nix/loader.nix { inherit pkgs; };
          };
        }
      );
    };
}
