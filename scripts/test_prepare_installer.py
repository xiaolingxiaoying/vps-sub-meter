import base64
from pathlib import Path
import runpy
import unittest


render = runpy.run_path(str(Path(__file__).with_name("prepare-installer.py")))["render"]


def read_pins():
    pins = {}
    for line in Path(__file__).with_name("release-runtime-pins.txt").read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        key, _, value = line.partition("=")
        pins[key.strip()] = value.strip()
    return pins


def version_tuple(value):
    parts = value.split(".")
    while len(parts) < 3:
        parts.append("0")
    return tuple(int(part) for part in parts)


class InstallerTrustTests(unittest.TestCase):
    def test_missing_malformed_and_public_development_keys_are_rejected(self):
        for key in ["", "invalid", "00" * 31, "247f88e163242986b7107eb704a983e12186d2697a3927b7e6b42ec2b364272b"]:
            with self.subTest(key_length=len(key)), self.assertRaises(ValueError):
                render(key)

    def test_installer_pins_the_supplied_public_key(self):
        # RFC 8032 public test vector; this is never a production key.
        key = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        script = render(key)
        self.assertNotIn("@SBCTL_RELEASE_PUBLIC_KEY_PEM@", script)
        self.assertIn(
            "https://github.com/xiaolingxiaoying/vps-sub-meter/releases/latest/download/manifest-{arch}.json",
            script,
        )
        encoded = script.split("-----BEGIN PUBLIC KEY-----\n", 1)[1].splitlines()[0]
        self.assertEqual(base64.b64decode(encoded)[-32:], bytes.fromhex(key))
        self.assertNotIn("JH+I4WMkKYa3EH63BKmD4SGG0ml6OSe35rQuwrNkJys=", script)


class RuntimePinTests(unittest.TestCase):
    """The release runtime pin is a signed-payload input, so it is checked."""

    def test_every_required_pin_is_present_and_well_formed(self):
        pins = read_pins()
        for key in [
            "sing_box_version",
            "sing_box_compat_min",
            "sing_box_compat_max",
            "sha256_amd64",
            "sha256_arm64",
        ]:
            self.assertIn(key, pins)
        self.assertRegex(pins["sing_box_version"], r"^\d+\.\d+\.\d+$")
        for arch in ["amd64", "arm64"]:
            self.assertRegex(pins[f"sha256_{arch}"], r"^[0-9a-f]{64}$")

    def test_the_compatibility_band_contains_the_pinned_runtime(self):
        pins = read_pins()
        version = version_tuple(pins["sing_box_version"])
        minimum = version_tuple(pins["sing_box_compat_min"])
        maximum = version_tuple(pins["sing_box_compat_max"])
        self.assertLessEqual(minimum, maximum)
        self.assertLessEqual(minimum, version)
        self.assertLessEqual(version, maximum)

    def test_the_manifest_generator_reads_the_same_pin_file(self):
        generator = Path(__file__).with_name("generate-manifest.sh").read_text(encoding="utf-8")
        self.assertIn("release-runtime-pins.txt", generator)
        self.assertIn("sing_box_compat_min", generator)


if __name__ == "__main__":
    unittest.main()
