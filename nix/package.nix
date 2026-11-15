{ pkgs, self, ... }:
let
  # The release workflow writes nix/version into the commit that the latest tag names.
  version =
    if builtins.pathExists ./version then
      pkgs.lib.trim (builtins.readFile ./version)
    else
      "0.0.0-${self.sourceInfo.shortRev or (self.sourceInfo.dirtyShortRev or "dirty")}";
in
pkgs.rustPlatform.buildRustPackage {
  pname = "ia-get";
  inherit version;
  src = self;
  cargoLock.lockFile = ../Cargo.lock;
  meta = {
    description = "File downloader for archive.org";
    homepage = "https://github.com/wimpysworld/ia-get";
    license = pkgs.lib.licenses.mit;
    mainProgram = "ia-get";
  };
}
