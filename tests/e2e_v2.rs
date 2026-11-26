#[path = "support/secrets.rs"]
mod test_secrets;

use std::fs;
use vps_sub_meter::{
    apply,
    config::{Config, DEFAULT_CONFIG_REL},
    render, store, traffic,
};

#[test]
fn full_sandbox_lifecycle_and_external_client_template() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // Seed fake sysfs & boot_id for traffic accounting
    let stats = root.join("sys/class/net/eth0/statistics");
    fs::create_dir_all(&stats).unwrap();
    fs::write(stats.join("rx_bytes"), "1048576").unwrap();
    fs::write(stats.join("tx_bytes"), "2097152").unwrap();
    let proc_dir = root.join("proc/sys/kernel/random");
    fs::create_dir_all(&proc_dir).unwrap();
    fs::write(proc_dir.join("boot_id"), "sandbox-boot-1").unwrap();

    // 1. Apply a configuration derived from customized deployed defaults.
    let defaults = root.join(DEFAULT_CONFIG_REL);
    fs::create_dir_all(defaults.parent().unwrap()).unwrap();
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/default-config.toml"),
        &defaults,
    )
    .unwrap();
    let mut initial = Config {
        domain: "sub.example.com".into(),
        public_ip: "203.0.113.20".into(),
        ..Config::load_defaults(root).unwrap()
    };
    initial.traffic.monthly_gb = 500.0;
    initial.traffic.reset_day = 11;
    initial.traffic.reset_time = "01:32".into();
    initial.nodes.vless_reality_pqc.decoy_sni = "decoy.example.com".into();
    fs::write(&defaults, initial.to_commented_toml()).unwrap();
    let sec = test_secrets::fixture(&initial);
    sec.save(root).unwrap();

    initial.save(root).unwrap();
    apply::apply_all(root).unwrap();
    let cfg = Config::load(root).unwrap();
    assert_eq!(cfg, initial);
    let generation = store::current_generation(root).unwrap().unwrap();
    assert_eq!(generation.direct_nodes.len(), 5);
    assert_eq!(generation.singbox_nodes.len(), 3);

    // 2. Verify traffic checkpoint + userinfo direction
    let snap = traffic::set_used(root, &cfg, 1000, 3000).unwrap();
    let hdr = snap.userinfo_header();
    assert!(hdr.starts_with("upload=1000; download=3000; total="));

    // 3. Verify external mihomo.yaml template override is respected
    let tpl_dir = root.join("etc/vps-sub-meter/templates");
    fs::create_dir_all(&tpl_dir).unwrap();
    let custom_mihomo = include_str!("../templates/mihomo.yaml").replace(
        "external-controller: ''",
        "external-controller: '127.0.0.1:9099'",
    );
    fs::write(tpl_dir.join("mihomo.yaml"), custom_mihomo).unwrap();
    let bundle = render::client::compile_direct_clients(root, &cfg, &sec).unwrap();
    assert!(bundle.clash_yaml.contains("127.0.0.1:9099"));
}
