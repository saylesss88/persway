{
  inputs,
  pkgs,
  ansiEscape,
  ...
}: rec {
  name = "Rust application";
  languages.rust.enable = true;
  languages.rust.toolchain = inputs.fenix.packages.${pkgs.stdenv.hostPlatform.system}.default.toolchain;
  languages.nix.enable = true;
  packages = with pkgs; [
    nixfmt
    taplo
  ];
  enterShell = ansiEscape ''
     echo -e "
      {bold}{160}${name}{reset}

      Persway - the evil, scheming, ipc daemon
    "
  '';
}
