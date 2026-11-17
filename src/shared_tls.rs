//! Optional TLS passthrough on the existing HTTPS listener. No TLS termination,
//! extra resident process, access log or on-disk traffic cache.
use serde::{Deserialize, Serialize};
use std::{io, net::SocketAddr, time::Duration};
use tokio::{io::copy_bidirectional, net::TcpStream};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub server_name: String,
    pub backend: SocketAddr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedTls {
    pub routes: Vec<Route>,
}
impl SharedTls {
    pub fn validate(&self, config: &crate::config::DeploymentConfig) -> Result<(), &'static str> {
        if config.subscription_mode != crate::config::SubscriptionMode::Direct {
            return Err(
                "443 SNI 分流需要现有自动 HTTPS 订阅入口；只有 IP 时可让 UDP 节点与网站 TCP 443 共用端口号",
            );
        }
        if self.routes.len() > 64 {
            return Err("443 分流最多 64 条路由");
        }
        let mut names = std::collections::BTreeSet::new();
        let mut nodes = std::collections::BTreeSet::new();
        for route in &self.routes {
            if !crate::config::hostname_is_valid(&route.server_name)
                || route
                    .server_name
                    .eq_ignore_ascii_case(&config.subscription_host)
                || !names.insert(route.server_name.to_ascii_lowercase())
                || !route.backend.ip().is_loopback()
                || route.backend.port() < 1024
                || route.backend.port() == 443
            {
                return Err(
                    "443 分流需要不同的有效 SNI；后端仅允许本机高端口，不能覆盖订阅域名或回指 443",
                );
            }
            if let Some(id) = &route.node_id
                && (!crate::nodes::identifier(id) || !nodes.insert(id))
            {
                return Err("443 节点路由 ID 无效或重复");
            }
        }
        Ok(())
    }
    pub fn node_route(&self, id: &str) -> Option<&Route> {
        self.routes
            .iter()
            .find(|r| r.node_id.as_deref() == Some(id))
    }
}

/// The bound listener differs from the advertised port only for a routed node.
pub fn binding(config: &crate::config::DeploymentConfig, node: &crate::nodes::Node) -> SocketAddr {
    if node.port() == 443
        && let Some(route) = config
            .shared_tls
            .as_ref()
            .and_then(|s| s.node_route(&node.id))
    {
        return route.backend;
    }
    SocketAddr::new(
        node.listen.parse().expect("validated listener address"),
        node.port(),
    )
}
pub fn sni(node: &crate::nodes::Node) -> &str {
    match &node.data {
        crate::nodes::Data::Legacy(crate::canonical::CanonicalNode::VlessReality {
            decoy_sni,
            ..
        }) => decoy_sni,
        crate::nodes::Data::Legacy(n) => n.tls_server_name(),
        crate::nodes::Data::Combination(n) => &n.server_name,
    }
}

/// Rustls parses the ClientHello, including fragmented TLS records. Peeking
/// preserves every original byte for the selected backend or HTTPS acceptor.
pub async fn server_name(stream: &TcpStream) -> io::Result<Option<String>> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut bytes = vec![0u8; 65536];
        let mut fed = 0;
        let mut acceptor = rustls::server::Acceptor::default();
        loop {
            let count = stream.peek(&mut bytes).await?;
            if count == 0 {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
            while count > fed {
                let consumed = acceptor.read_tls(&mut io::Cursor::new(&bytes[fed..count]))?;
                if consumed == 0 {
                    return Err(io::Error::other("incomplete TLS record"));
                }
                fed += consumed;
                match acceptor.accept() {
                    Ok(Some(accepted)) => {
                        return Ok(accepted.client_hello().server_name().map(str::to_owned));
                    }
                    Ok(None) => {}
                    Err(_) => return Err(io::Error::other("invalid TLS ClientHello")),
                }
            }
            if count == bytes.len() {
                return Err(io::Error::other("TLS ClientHello exceeds 64 KiB"));
            }
            // An incomplete peek leaves the socket readable; avoid a busy loop.
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}
pub async fn forward(mut client: TcpStream, backend: SocketAddr) -> io::Result<()> {
    let mut server = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(backend))
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
    client.set_nodelay(true)?;
    server.set_nodelay(true)?;
    tokio::time::timeout(
        Duration::from_secs(86400),
        copy_bidirectional(&mut client, &mut server),
    )
    .await
    .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    fn hello(name: &'static str) -> Vec<u8> {
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        let mut client = rustls::ClientConnection::new(
            std::sync::Arc::new(config),
            rustls::pki_types::ServerName::try_from(name).unwrap(),
        )
        .unwrap();
        let mut bytes = Vec::new();
        client.write_tls(&mut bytes).unwrap();
        bytes
    }
    #[tokio::test]
    async fn fragmented_client_hello_is_forwarded_byte_for_byte_to_only_the_selected_backend() {
        let bytes = hello("web.example.test");
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let backend_addr = backend.local_addr().unwrap();
        let expected = bytes.clone();
        let receiver = tokio::spawn(async move {
            let (mut stream, _) = backend.accept().await.unwrap();
            let mut received = vec![0; expected.len()];
            stream.read_exact(&mut received).await.unwrap();
            assert_eq!(received, expected);
            stream.write_all(b"forwarded").await.unwrap();
            stream.shutdown().await.unwrap();
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            assert_eq!(
                server_name(&stream).await.unwrap().as_deref(),
                Some("web.example.test")
            );
            forward(stream, backend_addr).await.unwrap();
        });
        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(&bytes[..7]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(15)).await;
        client.write_all(&bytes[7..]).await.unwrap();
        let mut response = [0; 9];
        tokio::time::timeout(Duration::from_secs(3), client.read_exact(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&response, b"forwarded");
        client.shutdown().await.unwrap();
        receiver.await.unwrap();
        router.await.unwrap();
    }
    #[tokio::test]
    async fn plaintext_is_rejected_without_turning_the_listener_into_an_open_tcp_proxy() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            assert!(server_name(&stream).await.is_err());
        });
        let mut client = TcpStream::connect(address).await.unwrap();
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
    }
}
