use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use otto_core::Id;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;

use super::interposer::{ensure_interposer_lib, wrap_shell_script_for_interposer};
use super::k8s_discover::{
    parse_k8s_discovery_json, resolve_destination, BridgeDestination, DiscoveryCache,
    K8sDiscoverySnapshot,
};
use super::socks5::{connect_via_socks5, connect_via_socks5_with_auth};
use super::tunnel::{vip_for_seq, NetBridgeInstance, MAX_VIP_ENTRIES};
use super::tunnel_forward::is_ssh_routable_private_ip;
use super::{sanitize_network_bridge, IdeK8sSpec};
use crate::types::{IdeHostMapping, IdeNetworkBridgeConfig};

fn sample_cbo_k8s_json() -> (serde_json::Value, serde_json::Value) {
    let svcs = json!({
        "items": [
            {
                "metadata": { "name": "confsrv" },
                "spec": {
                    "clusterIP": "10.96.42.10",
                    "ports": [{ "name": "http", "port": 9090, "targetPort": 9090, "protocol": "TCP" }]
                }
            },
            {
                "metadata": { "name": "discovery" },
                "spec": {
                    "clusterIP": "10.96.42.11",
                    "ports": [{ "name": "http", "port": 8080, "targetPort": 8080, "protocol": "TCP" }]
                }
            },
            {
                "metadata": { "name": "redis" },
                "spec": {
                    "clusterIP": "10.96.42.12",
                    "ports": [{ "name": "tcp", "port": 6379, "targetPort": 6379, "protocol": "TCP" }]
                }
            }
        ]
    });
    let ings = json!({
        "items": [
            {
                "spec": {
                    "rules": [
                        {
                            "host": "confsrv.stgco.local",
                            "http": {
                                "paths": [{
                                    "backend": {
                                        "service": { "name": "confsrv", "port": { "number": 9090 } }
                                    }
                                }]
                            }
                        }
                    ]
                }
            }
        ]
    });
    (svcs, ings)
}

#[test]
fn discovery_routes_cbo_config_server_service_dns_and_cluster_ip() {
    let (svcs, ings) = sample_cbo_k8s_json();
    let mut snap = parse_k8s_discovery_json("cbo", &svcs, &ings);
    let cfg = IdeNetworkBridgeConfig {
        enabled: true,
        k8s_cluster_id: Some(Id::from("01M3PSPFMHKRJ5RGDNKBC90ECH")),
        k8s_namespace: Some("cbo".into()),
        ..Default::default()
    };

    assert_eq!(
        resolve_destination("confsrv.stgco.local", 80, &cfg, Some(&snap), false),
        Some(BridgeDestination::K8sService {
            namespace: "cbo".into(),
            service: "confsrv".into(),
            port: 9090,
        })
    );
    assert_eq!(
        resolve_destination("discovery.stgco.local", 8080, &cfg, Some(&snap), false),
        Some(BridgeDestination::K8sService {
            namespace: "cbo".into(),
            service: "discovery".into(),
            port: 8080,
        })
    );
    assert_eq!(
        resolve_destination("discovery", 8080, &cfg, Some(&snap), false),
        Some(BridgeDestination::K8sService {
            namespace: "cbo".into(),
            service: "discovery".into(),
            port: 8080,
        })
    );
    assert_eq!(
        resolve_destination(
            "redis.cbo.svc.cluster.local",
            6379,
            &cfg,
            Some(&snap),
            false
        ),
        Some(BridgeDestination::K8sService {
            namespace: "cbo".into(),
            service: "redis".into(),
            port: 6379,
        })
    );
    assert_eq!(
        resolve_destination("10.96.42.10", 9090, &cfg, Some(&snap), false),
        Some(BridgeDestination::K8sService {
            namespace: "cbo".into(),
            service: "confsrv".into(),
            port: 9090,
        })
    );

    assert_eq!(
        resolve_destination(
            "kafka.infra.svc.cluster.local",
            9092,
            &cfg,
            Some(&snap),
            false
        ),
        None
    );
    snap.allow_cross_namespace = true;
    assert_eq!(
        resolve_destination(
            "kafka.infra.svc.cluster.local",
            9092,
            &cfg,
            Some(&snap),
            false
        ),
        Some(BridgeDestination::K8sService {
            namespace: "infra".into(),
            service: "kafka".into(),
            port: 9092,
        })
    );
}

#[test]
fn sanitize_validates_namespace_and_mappings() {
    let bad = IdeNetworkBridgeConfig {
        enabled: true,
        k8s_cluster_id: Some(Id::from("cluster-1")),
        k8s_namespace: Some("../bad_ns".into()),
        ..Default::default()
    };
    assert!(sanitize_network_bridge(Some(bad)).is_err());

    let empty_disabled = IdeNetworkBridgeConfig::default();
    assert_eq!(sanitize_network_bridge(Some(empty_disabled)).unwrap(), None);

    let mapped = IdeNetworkBridgeConfig {
        enabled: true,
        host_mappings: vec![IdeHostMapping {
            from_host: "LOCALHOST.".into(),
            from_port: Some(9090),
            to_host: "confsrv".into(),
            to_port: Some(9090),
        }],
        ..Default::default()
    };
    let clean = sanitize_network_bridge(Some(mapped)).unwrap().unwrap();
    assert_eq!(clean.host_mappings[0].from_host, "localhost");
}

#[test]
fn vip_base254_math_never_emits_zero_or_broadcast_and_classifies_ssh_private_ips() {
    assert_eq!(vip_for_seq(1), Ipv4Addr::new(127, 88, 1, 1));
    assert_eq!(vip_for_seq(254), Ipv4Addr::new(127, 88, 1, 254));
    assert_eq!(vip_for_seq(255), Ipv4Addr::new(127, 88, 2, 1));
    assert_eq!(vip_for_seq(256), Ipv4Addr::new(127, 88, 2, 2));
    assert_eq!(vip_for_seq(64516), Ipv4Addr::new(127, 88, 254, 254));
    assert_eq!(vip_for_seq(64517), Ipv4Addr::new(127, 88, 1, 1));

    for seq in [1u32, 254, 255, 256, 64516, 64517]
        .into_iter()
        .chain(1..=2048)
    {
        let oct = vip_for_seq(seq).octets();
        assert_eq!((oct[0], oct[1]), (127, 88));
        assert!(
            (1..=254).contains(&oct[2]) && (1..=254).contains(&oct[3]),
            "seq={seq} produced invalid octet in {oct:?}"
        );
    }

    for priv_ip in ["10.20.30.40", "172.16.5.9", "192.168.1.10", "fd00::1234"] {
        assert!(
            is_ssh_routable_private_ip(priv_ip.parse::<IpAddr>().unwrap()),
            "expected {priv_ip} to route via SSH tunnel"
        );
    }
    for non_ssh_ip in [
        "127.0.0.1",
        "127.88.1.1",
        "::1",
        "169.254.169.254",
        "100.100.100.200",
        "8.8.8.8",
    ] {
        assert!(
            !is_ssh_routable_private_ip(non_ssh_ip.parse::<IpAddr>().unwrap()),
            "expected {non_ssh_ip} NOT to match SSH private IP router"
        );
    }
}

#[tokio::test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn netbridge_interposes_cbo_config_and_enforces_socks5_auth_and_ssrf_guard() {
    let pod_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let pod_port = pod_listener.local_addr().unwrap().port();
    let pod_task = tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = pod_listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut req_buf = [0u8; 64];
                let _ = s.read(&mut req_buf).await;
                let _ = s.write_all(b"bridged-cbo-config-ok\n").await;
                let _ = s.shutdown().await;
            });
        }
    });

    let tmp = tempfile::tempdir().unwrap();
    let compiled = ensure_interposer_lib(tmp.path())
        .await
        .expect("compile C interposer");
    assert!(
        compiled.is_some(),
        "C interposer library must compile on macOS/Linux"
    );

    let kubectl_script = tmp.path().join("mock-kubectl.sh");
    let script_body = format!(
        r#"#!/bin/sh
case "$*" in
  *"get services"*)
    cat <<'EOF'
{{"items":[{{"metadata":{{"name":"confsrv"}},"spec":{{"clusterIP":"10.96.42.10","ports":[{{"name":"http","port":9090,"targetPort":9090,"protocol":"TCP"}}]}}}}]}}
EOF
    exit 0
    ;;
  *"get ingresses"*)
    cat <<'EOF'
{{"items":[{{"spec":{{"rules":[{{"host":"confsrv.stgco.local","http":{{"paths":[{{"backend":{{"service":{{"name":"confsrv","port":{{"number":9090}}}}}}}}]}}}}]}}}}]}}
EOF
    exit 0
    ;;
  *"port-forward service/confsrv 0:9090"*)
    exec python3 -u -c '
import socket, threading
pod_port = {pod_port}
srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.bind(("127.0.0.1", 0))
srv.listen(16)
lport = srv.getsockname()[1]
print(f"Forwarding from 127.0.0.1:{{lport}} -> 9090", flush=True)
def pump(a, b):
    try:
        while True:
            d = a.recv(4096)
            if not d: break
            b.sendall(d)
    except Exception:
        pass
    finally:
        # Half-close, never close(): on Linux a close() while the other pump
        # thread is blocked in recv() on the same socket sends no FIN, so the
        # client would wait for EOF forever.
        try: b.shutdown(socket.SHUT_WR)
        except Exception: pass
while True:
    c, _ = srv.accept()
    u = socket.create_connection(("127.0.0.1", pod_port))
    threading.Thread(target=pump, args=(c, u), daemon=True).start()
    threading.Thread(target=pump, args=(u, c), daemon=True).start()
'
    ;;
esac
exit 1
"#
    );
    tokio::fs::write(&kubectl_script, script_body)
        .await
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&kubectl_script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let cfg = IdeNetworkBridgeConfig {
        enabled: true,
        k8s_cluster_id: Some(Id::from("stg-m")),
        k8s_namespace: Some("cbo".into()),
        inject_k8s_service_env: true,
        containerized: true,
        host_mappings: Vec::new(),
        ssh_tunnel_connection_id: None,
    };
    let k8s_spec = IdeK8sSpec {
        cluster_id: Id::from("stg-m"),
        cluster_name: "STG-M".into(),
        environment: None,
        namespace: "cbo".into(),
        allow_cross_namespace: false,
        kubectl_program: kubectl_script.to_string_lossy().into_owned(),
        kubectl_query_args: Vec::new(),
        kubectl_base_args: Vec::new(),
        kubectl_env: Vec::new(),
    };

    let inst = NetBridgeInstance::start(
        tmp.path(),
        cfg,
        Some(k8s_spec.clone()),
        None,
        Arc::new(DiscoveryCache::default()),
    )
    .await
    .unwrap();

    // Verify 4096 VIP table bound.
    for i in 0..(MAX_VIP_ENTRIES + 128) {
        let _ = inst.allocate_vip(&format!("ephemeral-{i}.cbo.local"));
    }
    assert_eq!(inst.vip_count(), MAX_VIP_ENTRIES);

    let mut child_env = BTreeMap::new();
    inst.apply_to_env(&mut child_env);
    assert!(!child_env.contains_key("JAVA_TOOL_OPTIONS"));
    assert!(child_env.contains_key("OTTO_NETBRIDGE_DYLIB"));
    assert_eq!(
        child_env.get("OTTO_K8S_NAMESPACE").map(String::as_str),
        Some("cbo")
    );
    assert_eq!(
        child_env.get("CONTAINERIZED").map(String::as_str),
        Some("true")
    );
    assert!(child_env
        .get("CONFSRV_SERVICE_HOST")
        .is_some_and(|ip| ip.starts_with("127.88.")));
    assert_eq!(
        child_env.get("CONFSRV_SERVICE_PORT").map(String::as_str),
        Some("9090")
    );

    // Parse `socks5h://otto:{token}@127.0.0.1:{port}` and verify RFC 1929 auth.
    let socks_uri = child_env.get("OTTO_NETBRIDGE_SOCKS5").unwrap();
    let rest = socks_uri
        .strip_prefix("socks5h://otto:")
        .expect("socks5h://otto: scheme");
    let (tok_part, addr_part) = rest.split_once('@').expect("@ separator");
    assert_eq!(tok_part, inst.auth_token());
    let socks_port: u16 = addr_part.rsplit(':').next().unwrap().parse().unwrap();

    // 1) Unauthenticated greeting must be rejected with 0xFF.
    {
        let mut raw = TcpStream::connect(("127.0.0.1", socks_port)).await.unwrap();
        raw.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut rep = [0u8; 2];
        raw.read_exact(&mut rep).await.unwrap();
        assert_eq!(rep, [0x05, 0xFF]);
    }
    assert!(connect_via_socks5(socks_port, "confsrv.stgco.local", 9090)
        .await
        .is_err());

    // 2) Wrong password & wrong username must be rejected.
    assert!(connect_via_socks5_with_auth(
        socks_port,
        "confsrv.stgco.local",
        9090,
        Some(("otto", "wrong-token")),
    )
    .await
    .is_err());
    assert!(connect_via_socks5_with_auth(
        socks_port,
        "confsrv.stgco.local",
        9090,
        Some(("other", tok_part)),
    )
    .await
    .is_err());

    // 3) Loopback & cloud-metadata direct fallback via SOCKS5 must be blocked by netguard.
    assert!(connect_via_socks5_with_auth(
        socks_port,
        "127.0.0.1",
        pod_port,
        Some(("otto", tok_part)),
    )
    .await
    .is_err());
    assert!(connect_via_socks5_with_auth(
        socks_port,
        "169.254.169.254",
        80,
        Some(("otto", tok_part)),
    )
    .await
    .is_err());

    // 4) Valid `otto:{token}` connects to bridged K8s service `confsrv.stgco.local:9090`.
    let mut socks_stream = connect_via_socks5_with_auth(
        socks_port,
        "confsrv.stgco.local",
        9090,
        Some(("otto", tok_part)),
    )
    .await
    .unwrap();
    socks_stream
        .write_all(b"GET / HTTP/1.0\r\n\r\n")
        .await
        .unwrap();
    let mut reply = String::new();
    socks_stream.read_to_string(&mut reply).await.unwrap();
    assert_eq!(reply.trim(), "bridged-cbo-config-ok");

    // 5) Verify native POSIX C interposer (`DYLD_INSERT_LIBRARIES` / `LD_PRELOAD`).
    let probe_c = tmp.path().join("probe_client.c");
    let probe_bin = tmp.path().join("probe_client");
    tokio::fs::write(
        &probe_c,
        r#"
#include <sys/types.h>
#include <sys/socket.h>
#include <netdb.h>
#include <unistd.h>
#include <stdio.h>
#include <string.h>

int main(void) {
    struct addrinfo hints, *res = NULL;
    memset(&hints, 0, sizeof(hints));
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo("confsrv.stgco.local", "9090", &hints, &res) != 0 || !res) return 2;
    int fd = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (fd < 0) return 3;
    if (connect(fd, res->ai_addr, res->ai_addrlen) != 0) return 4;
    freeaddrinfo(res);
    (void)write(fd, "PING\n", 5);
    char buf[128];
    ssize_t n = read(fd, buf, sizeof(buf) - 1);
    close(fd);
    if (n <= 0) return 5;
    buf[n] = '\0';
    fputs(buf, stdout);
    return 0;
}
"#,
    )
    .await
    .unwrap();

    let cc_ok = Command::new("cc")
        .arg("-O2")
        .arg(&probe_c)
        .arg("-o")
        .arg(&probe_bin)
        .status()
        .await
        .is_ok_and(|s| s.success());
    assert!(cc_ok, "cc must compile probe_client.c");

    let wrapped =
        wrap_shell_script_for_interposer(&format!("'{}'", probe_bin.display()), &child_env);
    let out = Command::new("/bin/sh")
        .arg("-c")
        .arg(&wrapped)
        .envs(&child_env)
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "probe_client failed (code {:?}): stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "bridged-cbo-config-ok"
    );

    // 6) Verify SWR DiscoveryCache serves stale snapshot immediately and refreshes in background.
    let swr_cache = DiscoveryCache::default();
    swr_cache.insert_with_age_for_test(
        &k8s_spec,
        K8sDiscoverySnapshot {
            namespace: "cbo".into(),
            ..Default::default()
        },
        Duration::from_secs(60),
    );
    let stale_snap = swr_cache.get_or_fetch(&k8s_spec, false).await.unwrap();
    assert!(stale_snap.services.is_empty());
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let cur = swr_cache.get_or_fetch(&k8s_spec, false).await.unwrap();
        if cur.services.contains_key("confsrv") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "SWR background refresh did not populate confsrv"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    drop(inst);
    pod_task.abort();
}

#[tokio::test]
async fn corr_p1_6_routing_cloud_managed_unmapped_vip_port_cgnat_and_idle_listener_cleanup() {
    let (svcs, ings) = sample_cbo_k8s_json();
    let loaded_snap = parse_k8s_discovery_json("cbo", &svcs, &ings);
    assert!(loaded_snap.loaded);

    let cfg = IdeNetworkBridgeConfig {
        enabled: true,
        k8s_cluster_id: Some(Id::from("stg-m")),
        k8s_namespace: Some("cbo".into()),
        ssh_tunnel_connection_id: Some(Id::from("ssh-bastion")),
        ..Default::default()
    };

    // 1) Loaded snapshot + SSH tunnel must NOT blindly hijack unknown single-label hosts,
    // `localhost`, or Bonjour machine `.local` hostnames to K8sService:
    assert_eq!(
        resolve_destination("unknown-single-host", 5432, &cfg, Some(&loaded_snap), true),
        Some(BridgeDestination::SshHost {
            host: "unknown-single-host".into(),
            port: 5432,
        })
    );
    assert_eq!(
        resolve_destination("unknown-single-host", 5432, &cfg, Some(&loaded_snap), false),
        None
    );
    let unloaded_snap = K8sDiscoverySnapshot {
        namespace: "cbo".into(),
        loaded: false,
        ..Default::default()
    };
    assert_eq!(
        resolve_destination("bootstrap-svc", 8080, &cfg, Some(&unloaded_snap), false),
        Some(BridgeDestination::K8sService {
            namespace: "cbo".into(),
            service: "bootstrap-svc".into(),
            port: 8080,
        })
    );
    assert_eq!(
        resolve_destination("localhost", 8080, &cfg, Some(&unloaded_snap), false),
        None
    );
    assert_eq!(
        resolve_destination("macbook-pro.local", 8080, &cfg, Some(&loaded_snap), true),
        None
    );

    // 2) Cloud-managed DB/cache suffixes (`.cache.amazonaws.com`, `.rds.amazonaws.com`,
    // `.azure.com`, `.googleapis.com`) must NOT collide with in-cluster `redis` service:
    assert_eq!(
        resolve_destination(
            "redis.prod.use1.cache.amazonaws.com",
            6379,
            &cfg,
            Some(&loaded_snap),
            true
        ),
        Some(BridgeDestination::SshHost {
            host: "redis.prod.use1.cache.amazonaws.com".into(),
            port: 6379,
        })
    );
    assert_eq!(
        resolve_destination(
            "redis.prod.use1.cache.amazonaws.com",
            6379,
            &cfg,
            Some(&loaded_snap),
            false
        ),
        None
    );

    // 3) CGNAT `100.64.0.0/10` is present in both Rust `is_ssh_routable_private_ip` and C interposer:
    assert!(is_ssh_routable_private_ip(
        "100.64.12.34".parse::<IpAddr>().unwrap()
    ));
    assert!(super::interposer::INTERPOSER_C_SOURCE_FOR_TEST
        .contains("if ((ip_h & 0xFFC00000u) == 0x64400000u) return 1;"));
    assert!(super::interposer::INTERPOSER_C_SOURCE_FOR_TEST.contains("otto_nb_connect_unix"));

    // 4) Port-scoped HostMapping VIP connecting on an unmapped port connects directly
    // via a `0600` Unix relay socket instead of failing `NotFound`, and idle cleanup
    // releases `host_mappings` TCP listeners:
    let direct_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let direct_port = direct_listener.local_addr().unwrap().port();
    let direct_task = tokio::spawn(async move {
        if let Ok((mut s, _)) = direct_listener.accept().await {
            let _ = s.write_all(b"direct-unmapped-ok\n").await;
            let _ = s.shutdown().await;
        }
    });

    let probe_ephemeral = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let mapped_local_port = probe_ephemeral.local_addr().unwrap().port();
    drop(probe_ephemeral);

    let tmp = tempfile::tempdir().unwrap();
    let bridge_cfg = IdeNetworkBridgeConfig {
        enabled: true,
        host_mappings: vec![IdeHostMapping {
            from_host: "127.0.0.1".into(),
            from_port: Some(mapped_local_port),
            to_host: "127.0.0.1".into(),
            to_port: Some(direct_port),
        }],
        ..Default::default()
    };
    let inst = NetBridgeInstance::start(
        tmp.path(),
        bridge_cfg,
        None,
        None,
        Arc::new(DiscoveryCache::default()),
    )
    .await
    .unwrap();
    assert_eq!(inst.active_host_mapping_listener_count(), 1);

    let vip = inst.allocate_vip("127.0.0.1");
    let target = inst
        .resolve_connect_target(&vip.to_string(), direct_port)
        .await
        .expect("unmapped VIP port must not return NotFound");
    match target {
        Some(super::tunnel_forward::LocalForwardTarget::UnixSocket(upath)) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&upath).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o600);
            }
            let mut us = tokio::net::UnixStream::connect(&upath).await.unwrap();
            let mut out = String::new();
            us.read_to_string(&mut out).await.unwrap();
            assert_eq!(out.trim(), "direct-unmapped-ok");
        }
        _ => panic!("expected UnixSocket relay for unmapped VIP port"),
    }

    // Non-configured hostname resolving to loopback/private IP is blocked by DNS-rebinding netguard:
    let rebind_vip = inst.allocate_vip("localhost");
    let rebind_res = inst
        .resolve_connect_target(&rebind_vip.to_string(), direct_port)
        .await;
    assert!(
        matches!(rebind_res, Err(otto_core::Error::Forbidden(_))),
        "hostname resolving to loopback must be blocked by resolve_host_checked"
    );

    inst.stop_idle_listeners();
    assert_eq!(inst.active_host_mapping_listener_count(), 0);
    direct_task.abort();
}
