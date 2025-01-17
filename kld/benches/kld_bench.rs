extern crate criterion;
use anyhow::{Error, Result};
use criterion::{criterion_group, criterion_main, Criterion};
use kld::database::{migrate_database, LdkDatabase};

use lightning::ln::functional_test_utils::{
    create_announced_chan_between_nodes, create_chanmon_cfgs, create_network, create_node_cfgs,
    create_node_chanmgrs, send_payment,
};
use lightning::util::logger::Level::Warn;
use lightning::util::test_utils::TestChainMonitor;
use test_utils::{cockroach, test_settings, CockroachManager};

criterion_group! {
    name = benches;
    config = Criterion::default().significance_level(0.1).sample_size(10).measurement_time(std::time::Duration::from_secs(30));
    targets = bench_send_payment_two_nodes
}
criterion_main!(benches);

// We add wrapper functions like that to only unwrap in one place and still cleanup all ressources.
pub fn bench_send_payment_two_nodes(c: &mut Criterion) {
    send_payment_two_nodes(c).unwrap()
}

/// Send one payment between two nodes with two cockroach instances.
/// The functional_test_utils just calls the message handlers on each node, no network involved.
pub fn send_payment_two_nodes(c: &mut Criterion) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()?;

    let (_cockroach_0, db_0, _cockroach_1, db_1) = runtime.block_on(async {
        let mut settings_0 = test_settings(env!("CARGO_TARGET_TMPDIR"), "bench_1");
        let cockroach_0 = cockroach!(settings_0);
        let db_0 = LdkDatabase::new(&settings_0).await?;
        migrate_database(&settings_0).await;
        let mut settings_1 = test_settings(env!("CARGO_TARGET_TMPDIR"), "bench_2");
        let cockroach_1 = cockroach!(settings_1);
        migrate_database(&settings_1).await;
        let db_1 = LdkDatabase::new(&settings_1).await?;
        Ok::<(CockroachManager, LdkDatabase, CockroachManager, LdkDatabase), Error>((
            cockroach_0,
            db_0,
            cockroach_1,
            db_1,
        ))
    })?;

    let mut chanmon_cfgs = create_chanmon_cfgs(2);
    chanmon_cfgs[0].logger.enable(Warn);
    chanmon_cfgs[1].logger.enable(Warn);
    let mut node_cfgs = create_node_cfgs(2, &chanmon_cfgs);

    let chain_mon_0 = TestChainMonitor::new(
        Some(&chanmon_cfgs[0].chain_source),
        &chanmon_cfgs[0].tx_broadcaster,
        &chanmon_cfgs[0].logger,
        &chanmon_cfgs[0].fee_estimator,
        &db_0,
        node_cfgs[0].keys_manager,
    );
    let chain_mon_1 = TestChainMonitor::new(
        Some(&chanmon_cfgs[1].chain_source),
        &chanmon_cfgs[1].tx_broadcaster,
        &chanmon_cfgs[1].logger,
        &chanmon_cfgs[1].fee_estimator,
        &db_1,
        node_cfgs[1].keys_manager,
    );
    node_cfgs[0].chain_monitor = chain_mon_0;
    node_cfgs[1].chain_monitor = chain_mon_1;
    let node_chanmgrs = create_node_chanmgrs(2, &node_cfgs, &[None, None]);
    let nodes = create_network(2, &node_cfgs, &node_chanmgrs);

    let _ = create_announced_chan_between_nodes(&nodes, 0, 1);

    c.bench_function("send_payment_two_nodes", |b| {
        b.iter(|| {
            send_payment(&nodes[0], &vec![&nodes[1]][..], 1000);
        });
    });
    Ok(())
}
