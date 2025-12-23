{
  description = "Create development shell with `nix develop`";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
    }:
    let
      overlays = [ (import rust-overlay) ];
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system overlays;
      };
    in
    {
      formatter.${system} = nixpkgs.legacyPackages.${system}.nixfmt-tree;
      devShells.${system}.default = pkgs.mkShell {
        buildInputs = with pkgs; [
          (rust-bin.selectLatestNightlyWith (toolchain: toolchain.default))
          rust-analyzer
          sqlx-cli
          cargo-deny
          pkg-config
          openssl
        ];
        RUST_BACKTRACE = "full";
        env.RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
      };
    };
}
