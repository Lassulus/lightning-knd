{ craneLib
, lib
, clippy
, openssl
, bitcoind
, cockroachdb
, pkg-config
, self
}:
let
  paths = [
    "Cargo.toml"
    "Cargo.lock"
    "api"
    "kld"
    "benches"
    "settings"
    "test-utils"
  ];
  src = lib.cleanSourceWith {
    src = self;
    filter = path: _type: lib.any (p: lib.hasPrefix "${self}/${p}" path) paths;
  };
  cargoToml = "${src}/kld/Cargo.toml";
  buildInputs = [ openssl ];
  nativeBuildInputs = [ pkg-config ];
  cargoExtraArgs = "--workspace --all-features";
  outputHashes = {
    "https://github.com/Mic92/bdk?branch=backport-begin-batch-result" = "sha256-6DrNnzy2jYpkxiNReAkUl22Iz6au0+kmePmTXQxUsug=";
  };
  cargoArtifacts = craneLib.buildDepsOnly {
    inherit src cargoToml buildInputs nativeBuildInputs cargoExtraArgs outputHashes;
  };
in
craneLib.buildPackage {
  name = "kld";
  inherit src cargoToml cargoArtifacts buildInputs nativeBuildInputs outputHashes;
  cargoExtraArgs = "${cargoExtraArgs} --bins --examples --lib";
  passthru = {
    clippy = craneLib.cargoClippy {
      inherit src cargoToml cargoArtifacts buildInputs nativeBuildInputs cargoExtraArgs outputHashes;
      cargoClippyExtraArgs = "--all-targets --no-deps -- -D warnings";
    };
    benches = craneLib.mkCargoDerivation {
      inherit src cargoToml cargoArtifacts buildInputs nativeBuildInputs cargoExtraArgs outputHashes;
      buildPhaseCargoCommand = "cargo bench --no-run";
    };
    # having the tests seperate avoids having to run them on every package change.
    tests = craneLib.cargoTest {
      inherit src cargoToml cargoArtifacts buildInputs cargoExtraArgs outputHashes;
      nativeBuildInputs = nativeBuildInputs ++ [ bitcoind cockroachdb ];
    };
    inherit cargoArtifacts;
  };

  # we run tests in a seperate package
  doCheck = false;

  meta = with lib; {
    description = "Lightning Network Kuutamo Node Distribution";
    homepage = "https://github.com/kuutamolabs/kld";
    license = licenses.asl20;
    platforms = platforms.unix;
  };
}
