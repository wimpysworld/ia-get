# Managed by Tailor: nix/rust.nix
{ pkgs, ... }:
with pkgs;
[
  rustc
  cargo
  rustfmt
  clippy
  cargo-audit
]
