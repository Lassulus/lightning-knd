let
  makeNode = nodeName:
    { self, lib, config, ... }:
    let
      cfg = config.kuutamo.cockroachdb;
    in
    {
      imports = [ self.nixosModules.cockroachdb ];
      system.activationScripts.cockroachdb = lib.stringAfter [ "specialfs" "users" "groups" ] ''
        install -D -m444 ${./cockroach-certs/ca.crt} "${cfg.certsDir}/ca.crt"
        install -D -m400 -o cockroachdb ${./cockroach-certs + "/${nodeName}.crt"} "${cfg.certsDir}/node.crt"
        install -D -m400 -o cockroachdb ${./cockroach-certs + "/${nodeName}.key"} "${cfg.certsDir}/node.key"
        ${lib.optionalString (config.networking.hostName == "node1") ''
          install -D -m400 ${./cockroach-certs/client.root.crt} "${cfg.certsDir}/client.root.crt"
          install -D -m400 ${./cockroach-certs/client.root.key} "${cfg.certsDir}/client.root.key"
        ''}
      '';

      # Bank/TPC-C benchmarks take some memory to complete
      virtualisation.memorySize = 2048;

      kuutamo.cockroachdb.nodeName = nodeName;

      networking.extraHosts = ''
        192.168.1.1 db1
        192.168.1.2 db2
        192.168.1.3 db3
      '';
      kuutamo.cockroachdb.join = [ "db1" "db2" "db3" ];
    };

in
import ./lib.nix (_: {
  name = "cockroachdb";


  nodes = {
    node1 = makeNode "db1";
    node2 = makeNode "db2";
    node3 = makeNode "db3";
  };

  # NOTE: All the nodes must start in order and you must NOT use startAll, because
  # there's otherwise no way to guarantee that node1 will start before the others try
  # to join it.
  testScript = ''
    for node in node1, node2, node3:
        node.start()

    for node in node1, node2, node3:
        node.wait_for_unit("cockroachdb")

    node1.wait_until_succeeds("cockroach-sql sql -e 'SHOW ALL CLUSTER SETTINGS' >&2")
    certsdir = "/var/lib/cockroachdb-certs"
    url = f"postgres://localhost:5432?sslmode=verify-full&sslrootcert={certsdir}/ca.crt&sslcert={certsdir}/client.root.crt&sslkey={certsdir}/client.root.key"
    node1.succeed(
        f"cockroach-sql workload init bank '{url}' >&2",
        f"cockroach-sql workload run bank '{url}' --duration=1m >&2",
    )
  '';
})