use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::Arc;

use otto_core::Id;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::Command;

use super::interposer::wrap_shell_script_for_interposer;
use super::k8s_discover::{
    parse_k8s_discovery_json, resolve_destination, BridgeDestination, DiscoveryCache,
};
use super::socks5::connect_via_socks5;
use super::tunnel::NetBridgeInstance;
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

    // 1) Ingress host (`confsrv.stgco.local` on default HTTP 80 -> service port 9090).
    assert_eq!(
        resolve_destination("confsrv.stgco.local", 80, &cfg, Some(&snap), false),
        Some(BridgeDestination::K8sService {
            namespace: "cbo".into(),
            service: "confsrv".into(),
            port: 9090,
        })
    );

    // 2) Smart `.local` first-label match (`discovery.stgco.local:8080` -> `service/discovery:8080`).
    assert_eq!(
        resolve_destination("discovery.stgco.local", 8080, &cfg, Some(&snap), false),
        Some(BridgeDestination::K8sService {
            namespace: "cbo".into(),
            service: "discovery".into(),
            port: 8080,
        })
    );

    // 3) Short service name & full FQDN.
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

    // 4) ClusterIP direct lookup.
    assert_eq!(
        resolve_destination("10.96.42.10", 9090, &cfg, Some(&snap), false),
        Some(BridgeDestination::K8sService {
            namespace: "cbo".into(),
            service: "confsrv".into(),
            port: 9090,
        })
    );

    // 5) Cross-namespace blocked when `allow_cross_namespace` is false, allowed when true.
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

#[tokio::test]
async fn netbridge_interposes_cbo_config_and_socks5_end_to_end() {
    if !cfg!(any(target_os = "macos", target_os = "linux")) {
        return;
    }

    // 1. Start a local mock "Cluster Pod" TCP responder representing `confsrv:9090`.
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

    // 2. Create a mock `kubectl` script serving `get services`, `get ingresses`,
    //    and `port-forward service/confsrv 0:9090` (forwarding to `pod_port`).
    let tmp = tempfile::tempdir().unwrap();
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
import socket, sys, threading
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
        try: a.close()
        except Exception: pass
        try: b.close()
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
        Some(k8s_spec),
        None,
        Arc::new(DiscoveryCache::default()),
    )
    .await
    .unwrap();

    let mut child_env = BTreeMap::new();
    inst.apply_to_env(&mut child_env);
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

    // 3. Verify JVM SOCKS5 proxy path (`confsrv.stgco.local:9090` -> mock pod).
    let socks_addr = child_env.get("OTTO_NETBRIDGE_SOCKS5").unwrap();
    let socks_port: u16 = socks_addr.rsplit(':').next().unwrap().parse().unwrap();
    let mut socks_stream = connect_via_socks5(socks_port, "confsrv.stgco.local", 9090)
        .await
        .unwrap();
    socks_stream
        .write_all(b"GET / HTTP/1.0\r\n\r\n")
        .await
        .unwrap();
    let mut reply = String::new();
    socks_stream.read_to_string(&mut reply).await.unwrap();
    assert_eq!(reply.trim(), "bridged-cbo-config-ok");

    // 4. Verify native POSIX C interposer (`DYLD_INSERT_LIBRARIES` / `LD_PRELOAD`)
    //    spawned through `/bin/sh -c` (`wrap_shell_script_for_interposer`).
    if child_env.contains_key("OTTO_NETBRIDGE_DYLIB") {
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
    if (getaddrinfo("confsrv.stgco.local", "9090", &hints, &res) != 0 || !res) {
        return 2;
    }
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
        if cc_ok {
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
        }
    }

    drop(inst);
    pod_task.abort();
}
