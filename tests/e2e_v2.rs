use std::fs;
use vps_sub_meter::{
    apply, compiler, config::Config, menu, prompt::BufferedIo, secrets::Secrets, traffic,
};

#[test]
fn full_sandbox_lifecycle_apply_config_edit_and_menu_zero_cancel() {
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

    // 1. Save domain config + test secrets + apply
    let mut cfg = Config {
        domain: "sub.example.com".into(),
        public_ip: "203.0.113.20".into(),
        ..Default::default()
    };
    cfg.traffic.monthly_gb = 500.0;
    cfg.traffic.reset_day = 11;
    cfg.traffic.reset_time = "01:32".into();
    cfg.save(root).unwrap();

    let sec = Secrets::generate_for_test(&cfg);
    sec.save(root).unwrap();

    let rep = apply::apply_all(root).unwrap();
    assert_eq!(rep.direct_nodes, 5);
    assert_eq!(rep.singbox_nodes, 3);

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
    let bundle = compiler::compile_direct_clients(root, &cfg, &sec).unwrap();
    assert!(bundle.clash_yaml.contains("127.0.0.1:9099"));

    // 4. Verify menu handles `0` cleanly without blocking or panicking
    let mut io = BufferedIo::new(std::io::Cursor::new("0\n"), Vec::new());
    menu::run(root, &mut io);
    let screen = String::from_utf8(io.writer).unwrap();
    assert!(screen.contains("vps-sub-meter"));
    assert!(screen.contains("0. 退出"));
}
