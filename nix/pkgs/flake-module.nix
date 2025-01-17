{ self, inputs, ... }: {
  perSystem = { config, self', pkgs, system, ... }: {
    packages = {
      kld = pkgs.callPackage ./kld.nix {
        inherit self;
        craneLib = inputs.crane.lib.${system};
        inherit (config.packages) cockroachdb;
      };
      kld-mgr = pkgs.callPackage ./kld-mgr.nix {
        inherit self;
      };
      remote-pdb = pkgs.python3.pkgs.callPackage ./remote-pdb.nix { };
      bitcoind = pkgs.bitcoind.override { withGui = false; };
      cockroachdb = pkgs.callPackage ./cockroachdb.nix { };
      default = self'.packages.kld-mgr;
    };
  };
}
