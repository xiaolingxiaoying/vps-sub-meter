import json
from pathlib import Path
import runpy
import tempfile
import unittest

MOD_PREPARE = runpy.run_path(str(Path(__file__).with_name("prepare-installer.py")))
MOD_MIGRATE = runpy.run_path(str(Path(__file__).with_name("migrate_legacy.py")))
run_migration = MOD_MIGRATE["run_migration"]


class UpgradeAndMigrationTests(unittest.TestCase):
    def test_prepare_installer_pins_key_into_upgrade_sh(self):
        key = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        rendered = MOD_PREPARE["render"](key, Path(__file__).with_name("upgrade.sh"))
        self.assertNotIn("@VSM_RELEASE_PUBLIC_KEY_PEM@", rendered)
        self.assertIn("-----BEGIN PUBLIC KEY-----", rendered)
        self.assertIn("migrate_legacy.py", rendered)

    def test_migrates_gen1_sub_srv_config_and_traffic(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "etc/sub-srv").mkdir(parents=True)
            (root / "var/lib/subsrv").mkdir(parents=True)
            (root / "sys/class/net/eth0/statistics").mkdir(parents=True)
            (root / "proc/sys/kernel/random").mkdir(parents=True)

            (root / "etc/sub-srv/config.conf").write_text(
                "\n".join(
                    [
                        "DOMAIN='sub.example.com'",
                        "TRAFFIC_LIMIT_GIB='500'",
                        "TZ_NAME='America/Los_Angeles'",
                        "RESET_MODE='anchored'",
                        "RESET_ANCHOR_DATE='2026-10-11'",
                        "RESET_HOUR='1'",
                        "RESET_MINUTE='32'",
                        "IFACE='eth0'",
                        "TOKEN='tok_v1_example_abcdef1234567890'",
                    ]
                ),
                encoding="utf-8",
            )
            (root / "var/lib/subsrv/tx_state.json").write_text(
                json.dumps({"base_rx": 1000, "base_tx": 2000}), encoding="utf-8"
            )
            (root / "sys/class/net/eth0/statistics/rx_bytes").write_text("5000", encoding="utf-8")
            (root / "sys/class/net/eth0/statistics/tx_bytes").write_text("9000", encoding="utf-8")
            (root / "proc/sys/kernel/random/boot_id").write_text("boot-gen1-test\n", encoding="utf-8")

            res = run_migration(root, check_only=False)
            self.assertEqual(res["generation"], "sub-srv-v1")
            self.assertTrue(res["migrated"])

            cfg_text = (root / "etc/vps-sub-meter/config.toml").read_text(encoding="utf-8")
            self.assertIn('domain = "sub.example.com"', cfg_text)
            self.assertIn("monthly_gb = 500.00", cfg_text)
            self.assertIn("reset_day = 11", cfg_text)
            self.assertIn('reset_time = "01:32"', cfg_text)

            sec = json.loads((root / "etc/vps-sub-meter/secrets.json").read_text(encoding="utf-8"))
            self.assertEqual(len(sec["tokens"]), 1)
            self.assertEqual(sec["tokens"][0]["token"], "tok_v1_example_abcdef1234567890")

            tx = json.loads((root / "var/lib/vps-sub-meter/traffic-state.json").read_text(encoding="utf-8"))
            self.assertEqual(tx["accumulated_rx"], 4000)
            self.assertEqual(tx["accumulated_tx"], 7000)

    def test_migrates_gen2_sbctl_filters_revoked_tokens_and_preserves_state(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "etc/sbctl").mkdir(parents=True)
            (root / "var/lib/sbctl").mkdir(parents=True)
            (root / "sys/class/net/eth0/statistics").mkdir(parents=True)
            (root / "proc/sys/kernel/random").mkdir(parents=True)

            (root / "etc/sbctl/config.toml").write_text(
                """
subscription_mode = "direct"
subscription_host = "203.0.113.10"
interface = "eth0"
ipv4_only = true
http_port = 2080
subscription_credential = "active_primary_token_1234567890"
monthly_traffic_limit = 536870912000
accounting_policy = "anchored-month"
accounting_timezone = "America/New_York"
client_display_timezone = "Asia/Shanghai"
anchored_reset_at = "2026-10-15T04:05"
reality_decoy_sni = "gateway.icloud.com"

[[subscription_credentials]]
credential = "active_extra_token_1234567890"
name = "laptop"

[[subscription_credentials]]
credential = "revoked_old_token_1234567890"
name = "lost-phone"
revoked_at = 1791872071

[vless_reality]
listen_port = 24443
uuid = "11111111-2222-3333-4444-555555555555"
private_key = "priv_test_key"
public_key = "pub_test_key"
short_id = "01020304"

[anytls]
listen_port = 36559
password = "anytls_pw_example"

[hysteria2]
listen_port = 32896
password = "hy2_pw_example"

[tuic]
listen_port = 58714
uuid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
password = "tuic_pw_example"
""",
                encoding="utf-8",
            )
            (root / "proc/sys/kernel/random/boot_id").write_text("boot-gen2-id\n", encoding="utf-8")
            (root / "sys/class/net/eth0/statistics/rx_bytes").write_text("1500", encoding="utf-8")
            (root / "sys/class/net/eth0/statistics/tx_bytes").write_text("2500", encoding="utf-8")
            (root / "var/lib/sbctl/state.json").write_text(
                json.dumps(
                    {
                        "boot_id": "boot-gen2-id",
                        "baseline_rx": 500,
                        "baseline_tx": 1000,
                        "accumulated_rx": 10000,
                        "accumulated_tx": 20000,
                    }
                ),
                encoding="utf-8",
            )

            res = run_migration(root, check_only=False)
            self.assertEqual(res["generation"], "sbctl")
            self.assertTrue(res["migrated"])

            cfg_text = (root / "etc/vps-sub-meter/config.toml").read_text(encoding="utf-8")
            self.assertIn('domain = ""', cfg_text)
            self.assertIn('public_ip = "203.0.113.10"', cfg_text)
            self.assertIn("ipv4_only = true", cfg_text)
            self.assertIn("reset_day = 15", cfg_text)
            self.assertIn('decoy_sni = "aws.amazon.com"', cfg_text)
            self.assertIn('reset_time = "04:05"', cfg_text)

            sec = json.loads((root / "etc/vps-sub-meter/secrets.json").read_text(encoding="utf-8"))
            tok_vals = [t["token"] for t in sec["tokens"]]
            self.assertEqual(
                tok_vals,
                ["active_primary_token_1234567890", "active_extra_token_1234567890"],
            )
            self.assertNotIn("revoked_old_token_1234567890", tok_vals)
            self.assertEqual(sec["vless_reality_pqc"]["x25519_private_key"], "priv_test_key")

            tx = json.loads((root / "var/lib/vps-sub-meter/traffic-state.json").read_text(encoding="utf-8"))
            self.assertEqual(tx["accumulated_rx"], 11000)
            self.assertEqual(tx["accumulated_tx"], 21500)

    def test_v2_deployment_is_untouched(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "etc/vps-sub-meter").mkdir(parents=True)
            (root / "etc/vps-sub-meter/config.toml").write_text('domain = "sub.example.com"\n', encoding="utf-8")
            res = run_migration(root, check_only=False)
            self.assertEqual(res, {"generation": "v2", "migrated": False})


if __name__ == "__main__":
    unittest.main()
