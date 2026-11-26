use vps_sub_meter::{
    config::Config,
    secrets::{
        ArgoWsSecret, Hysteria2Secrets, Secrets, SubscriptionToken, TuicSecrets,
        VlessRealitySecrets, VlessXhttpSecrets, generate_x25519, random_uuid,
    },
};

pub fn fixture(config: &Config) -> Secrets {
    let mut s = Secrets::default();
    s.tokens.push(SubscriptionToken {
        name: "default".into(),
        token: "test_token_0123456789abcdef0123456789".into(),
        created_at: 1_700_000_000,
    });
    let (priv_k, pub_k) = generate_x25519().unwrap();
    s.vless_reality_pqc = VlessRealitySecrets {
        uuid: random_uuid().unwrap(),
        x25519_private_key: priv_k,
        x25519_public_key: pub_k,
        short_id: "0123456789abcdef".into(),
        mldsa65_seed: "TEST_MLDSA65_SEED_43_CHARS_PLACEHOLDER_1234".into(),
        mldsa65_verify: "TEST_MLDSA65_VERIFY_PLACEHOLDER".into(),
        vless_decryption: "mlkem768x25519plus.native.600s.TEST_DEC_KEY".into(),
        vless_encryption: "mlkem768x25519plus.native.0rtt.TEST_ENC_KEY".into(),
    };
    s.vless_xhttp_pqc = VlessXhttpSecrets {
        uuid: random_uuid().unwrap(),
        path: "/test-xhttp-path".into(),
        vless_decryption: "mlkem768x25519plus.native.600s.TEST_XHTTP_DEC".into(),
        vless_encryption: "mlkem768x25519plus.native.0rtt.TEST_XHTTP_ENC".into(),
    };
    s.anytls_tcp.password = "anytls_test_pass".into();
    s.hysteria2_hop = Hysteria2Secrets {
        password: "hy2_test_pass".into(),
        obfs_password: "hy2_obfs_pass".into(),
    };
    s.tuic_bbr = TuicSecrets {
        uuid: random_uuid().unwrap(),
        password: "tuic_test_pass".into(),
    };
    for ws in &config.argo.ws_inbounds {
        s.argo_ws.insert(
            ws.id.clone(),
            ArgoWsSecret {
                uuid: random_uuid().unwrap(),
                path: format!("/ws-{}", ws.id),
            },
        );
    }
    s
}
