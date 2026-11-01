{ pkgs ? import <nixpkgs> { }, withGui ? true }:
pkgs.callPackage ./nix/package.nix {
  inherit withGui;
  runtimeLibraries = import ./nix/runtime-libraries.nix pkgs;
}
