(import ./lib.nix) ({ self, pkgs, ... }: {
  name = "from-nixos";
  nodes = {
    # self here is set by using specialArgs in `lib.nix`
    db1 = { self, ... }: {
      imports = [ self.nixosModules.kld ];
      # use the same name as the cert
      kuutamo.cockroachdb.nodeName = "db1";

      kuutamo.cockroachdb.caCertPath = ./cockroach-certs/ca.crt;
      kuutamo.cockroachdb.nodeCertPath = ./cockroach-certs + "/db1.crt";
      kuutamo.cockroachdb.nodeKeyPath = ./cockroach-certs + "/db1.key";
      kuutamo.cockroachdb.rootClientCertPath = ./cockroach-certs + "/client.root.crt";
      kuutamo.cockroachdb.rootClientKeyPath = ./cockroach-certs + "/client.root.key";

      kuutamo.kld.cockroachdb.clientCertPath = ./cockroach-certs + "/client.kld.crt";
      kuutamo.kld.cockroachdb.clientKeyPath = ./cockroach-certs + "/client.kld.key";

      kuutamo.kld.caPath = ./kld-certs/ca.pem;
      kuutamo.kld.certPath = ./kld-certs/kld.pem;
      kuutamo.kld.keyPath = ./kld-certs/kld.key;
      kuutamo.kld.network = "regtest";
    };
  };

  extraPythonPackages = _p: [ self.packages.${pkgs.system}.remote-pdb ];

  # This test is still wip
  testScript = ''
    start_all()

    # wait for our service to start
    db1.wait_for_unit("cockroachdb.service")
    db1.wait_for_unit("bitcoind-kld-regtest.service")
    db1.wait_for_unit("kld.service")

    db1.succeed("kld-bitcoin-cli createwallet testwallet >&2")
    address = db1.succeed("kld-bitcoin-cli getnewaddress").strip()
    db1.succeed(f"kld-bitcoin-cli generatetoaddress 1 {address}")
    # FIXME this block forever just now
    #out = db1.wait_until_succeeds("kld-cli get-info")

    # useful for debugging
    def remote_shell(machine):
        machine.shell_interact("tcp:127.0.0.1:4444,forever,interval=2")

    #remote_shell(machine)
    #breakpoint()
  '';
})
