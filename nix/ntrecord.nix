{ pkgs, ... }:
let
  version = "0.3.0";
  sources = {
    x86_64-linux = {
      url = "https://github.com/NimbleTerminal/ntrecord/releases/download/v${version}/ntrecord_${version}_linux_amd64.tar.gz";
      sha256 = "69f4da288202f38727bce4c190b9d16f792376662304cdeacbfef4e7422fc45b";
    };
    aarch64-linux = {
      url = "https://github.com/NimbleTerminal/ntrecord/releases/download/v${version}/ntrecord_${version}_linux_arm64.tar.gz";
      sha256 = "cb982892a4723662471c65a3c39588463fb84f37e1de70ab8fc49350f0819520";
    };
    aarch64-darwin = {
      url = "https://github.com/NimbleTerminal/ntrecord/releases/download/v${version}/ntrecord_${version}_darwin_arm64.tar.gz";
      sha256 = "6cf7446d9b714bfabd1e28632aeceea8db8f6c7e3930ab698bc8ce10c5158059";
    };
    x86_64-darwin = {
      url = "https://github.com/NimbleTerminal/ntrecord/releases/download/v${version}/ntrecord_${version}_darwin_amd64.tar.gz";
      sha256 = "93703adec24a6edf656ec024db480f0e2b78c37f277a2fc319b410fd25b539bf";
    };
  };
  system = pkgs.stdenv.hostPlatform.system;
  source = sources.${system} or (throw "ntrecord: unsupported system ${system}");
in
pkgs.stdenvNoCC.mkDerivation {
  pname = "ntrecord";
  inherit version;

  src = pkgs.fetchurl {
    inherit (source) url sha256;
  };

  sourceRoot = ".";
  dontConfigure = true;
  dontBuild = true;
  dontFixup = true;

  installPhase = ''
    runHook preInstall
    install -Dm755 ntrecord "$out/bin/ntrecord"
    runHook postInstall
  '';

  meta = {
    description = "Nimble Terminal recorder tool, supports Charm VHS Tapes and Kitty Graphics";
    homepage = "https://github.com/NimbleTerminal/ntrecord";
    license = pkgs.lib.licenses.mit;
    mainProgram = "ntrecord";
    platforms = builtins.attrNames sources;
  };
}
