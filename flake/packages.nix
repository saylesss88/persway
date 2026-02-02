{ inputs, ... }:
{
  perSystem =
    { pkgs, ... }:
    let
      toolchain = pkgs.fenix.complete.toolchain;
      craneLib = (inputs.crane.mkLib pkgs).overrideToolchain toolchain;
    in
    {
      packages.default = craneLib.buildPackage {
        src = craneLib.cleanCargoSource (craneLib.path ../.);
      };
    };
}
