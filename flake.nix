{
  description = "trainmaster の開発シェル(mise + SDL2)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # 現行 nixpkgs の SDL2 は sdl2-compat(SDL3 ベースの互換層)で、
    # レンダリングの挙動が変わりスナップショット検査(±2 許容)が通らない。
    # classic な SDL2 がまだある nixpkgs を pin して使う。
    nixpkgs-sdl2.url = "github:NixOS/nixpkgs/nixos-24.11";
  };

  outputs = { self, nixpkgs, nixpkgs-sdl2 }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          pkgs-sdl2 = nixpkgs-sdl2.legacyPackages.${system};
          # CI(ubuntu-24.04 の libsdl2-dev)と同じ版に合わせる
          sdl2-ci = pkgs-sdl2.SDL2.overrideAttrs (old: {
            version = "2.30.0";
            src = pkgs-sdl2.fetchurl {
              url = "https://github.com/libsdl-org/SDL/archive/release-2.30.0.tar.gz";
              hash = "sha256-tL4ZtwPL9lqKEfCjf+7pxdBkI46xn1zyN+DY1q8cwKc=";
            };
          });
        in
        {
          default = pkgs.mkShell {
            packages = [
              pkgs.mise
              sdl2-ci
              pkgs.pkg-config
            ];

            # mise のツール(rust など)は mise.toml が管理するので、ここでは入れない
            # (check:rustc が「実際に使われる rustc が mise.toml の版か」を検査するため)
          };
        });
    };
}
