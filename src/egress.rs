//! Bounded, explicit egress discovery for setup. Never runs on status/menu reads.
use std::{net::IpAddr, path::Path, process::Stdio};
pub fn discover(root: &Path) -> Result<IpAddr, String> {
    let runtime = crate::runtime::Runtime::live(root);
    for endpoint in ["https://api.ipify.org", "https://api64.ipify.org"] {
        let Ok(mut command) = runtime.command("curl", &[]) else {
            return Err("无法运行 curl；请填写 VPS 公网 IP".into());
        };
        let output = command
            .args([
                "--disable",
                "--noproxy",
                "*",
                "--fail",
                "--silent",
                "--show-error",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--connect-timeout",
                "3",
                "--max-time",
                "5",
                "--max-filesize",
                "128",
                endpoint,
            ])
            .stdin(Stdio::null())
            .output();
        if let Ok(output) = output
            && output.status.success()
            && let Ok(value) = std::str::from_utf8(&output.stdout)
            && let Ok(ip) = value.trim().parse::<IpAddr>()
            && public_ip(ip)
        {
            return Ok(ip);
        }
    }
    Err("未检测到公网出口 IP；请手动填写公网 IPv4 / IPv6（NAT 主机还需配置端口映射）".into())
}
pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && !ip.is_multicast()
                && !ip.is_unspecified()
                && ip.octets()[0] != 0
                && !(ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]))
                && ip.octets()[0] < 240
        }
        IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_multicast()
                && !ip.is_unspecified()
                && (ip.segments()[0] & 0xe000) == 0x2000
                && ip.segments()[0..2] != [0x2001, 0x0db8]
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn rejects_local_and_documentation_addresses() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "203.0.113.1",
            "::1",
            "fc00::1",
            "2001:db8::1",
        ] {
            assert!(!super::public_ip(ip.parse().unwrap()));
        }
        assert!(super::public_ip("1.1.1.1".parse().unwrap()));
        assert!(super::public_ip("2606:4700:4700::1111".parse().unwrap()));
    }
}
