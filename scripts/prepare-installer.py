#!/usr/bin/env python3
"""Pin the build-time Ed25519 public key into published install.sh / upgrade.sh."""
import argparse
import base64
import os
from pathlib import Path

DEV_KEY_HEX = "247f88e163242986b7107eb704a983e12186d2697a3927b7e6b42ec2b364272b"


def render(public_hex: str, template_path: Path | None = None) -> str:
    key = bytes.fromhex(public_hex.strip())
    if len(key) != 32 or key.hex() == DEV_KEY_HEX:
        raise ValueError("a new 32-byte production public key is required")
    der = bytes.fromhex("302a300506032b6570032100") + key
    pem = (
        "-----BEGIN PUBLIC KEY-----\n"
        + base64.b64encode(der).decode()
        + "\n-----END PUBLIC KEY-----"
    )
    src = template_path or Path(__file__).with_name("install.sh")
    template = src.read_text(encoding="utf-8")
    return template.replace("@VSM_RELEASE_PUBLIC_KEY_PEM@", pem)


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--template", default=str(Path(__file__).with_name("install.sh")))
    ap.add_argument("output")
    args = ap.parse_args()
    pub = os.environ.get("VSM_RELEASE_PUBLIC_KEY_HEX") or os.environ["SBCTL_RELEASE_PUBLIC_KEY_HEX"]
    output = Path(args.output)
    output.write_text(render(pub, Path(args.template)), encoding="utf-8", newline="\n")
