//! Native POSIX `getaddrinfo` + `connect` interposer library builder for macOS
//! (`DYLD_INSERT_LIBRARIES` `__DATA,__interpose`) and Linux (`LD_PRELOAD`).
//!
//! Why this exists:
//! Microservices in local IDE Run/Debug sessions frequently connect to internal
//! Kubernetes DNS names (`config:9090`, `confsrv.stgco.local`,
//! `redis.cbo.svc.cluster.local`) or private VPC hosts without proxy support.
//! By allocating synthetic `127.88.x.y` loopback VIPs in `getaddrinfo` and
//! rewriting `connect(127.88.x.y:port)` to an on-demand `127.0.0.1:<ephemeral>`
//! `kubectl port-forward` / SSH SOCKS5 forwarder over a `0600` Unix control
//! socket, unmodified Go, Rust, Python, Node.js, C/C++, and non-SIP binaries
//! reach cluster-internal services with zero `sudo` or `/etc/hosts` edits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use otto_core::{Error, Result};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use tokio::sync::Mutex;

static BUILD_LOCK: Mutex<()> = Mutex::const_new(());

const INTERPOSER_C_SRC: &str = r#"
#if !defined(__APPLE__)
#define _GNU_SOURCE
#endif

#include <sys/types.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/time.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <netdb.h>
#include <unistd.h>
#include <fcntl.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <strings.h>
#include <errno.h>
#include <stdint.h>

#if !defined(__APPLE__)
#include <dlfcn.h>
#endif

static int otto_nb_is_safe_host(const char *s) {
    if (!s || !s[0]) return 0;
    size_t n = 0;
    for (; s[n] != '\0'; n++) {
        unsigned char c = (unsigned char)s[n];
        if (n >= 253) return 0;
        if (c <= 0x20 || c >= 0x7f) return 0;
    }
    return n > 0;
}

static int otto_nb_query(const char *sock_path, const char *req, char *resp, size_t resp_cap) {
    if (!sock_path || !sock_path[0] || !req || !resp || resp_cap < 8) return -1;
    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    size_t path_len = strlen(sock_path);
    if (path_len == 0 || path_len >= sizeof(addr.sun_path)) return -1;
    memcpy(addr.sun_path, sock_path, path_len + 1);

#ifdef SOCK_CLOEXEC
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
#else
    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
#endif
    if (fd < 0) return -1;
    (void)fcntl(fd, F_SETFD, FD_CLOEXEC);

#ifdef SO_NOSIGPIPE
    int one = 1;
    (void)setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &one, sizeof(one));
#endif
    struct timeval tv;
    tv.tv_sec = 8;
    tv.tv_usec = 0;
    (void)setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv));
    (void)setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));

    if (connect(fd, (const struct sockaddr *)&addr, (socklen_t)sizeof(addr)) != 0) {
        close(fd);
        return -1;
    }

    size_t req_len = strlen(req);
    size_t off = 0;
    while (off < req_len) {
#ifdef MSG_NOSIGNAL
        ssize_t w = send(fd, req + off, req_len - off, MSG_NOSIGNAL);
#else
        ssize_t w = send(fd, req + off, req_len - off, 0);
#endif
        if (w < 0) {
            if (errno == EINTR) continue;
            close(fd);
            return -1;
        }
        off += (size_t)w;
    }

    size_t roff = 0;
    while (roff + 1 < resp_cap) {
        ssize_t r = recv(fd, resp + roff, resp_cap - 1 - roff, 0);
        if (r < 0) {
            if (errno == EINTR) continue;
            close(fd);
            return -1;
        }
        if (r == 0) break;
        roff += (size_t)r;
        if (memchr(resp, '\n', roff) != NULL) break;
    }
    close(fd);
    resp[roff] = '\0';
    while (roff > 0 && (resp[roff - 1] == '\n' || resp[roff - 1] == '\r')) {
        resp[--roff] = '\0';
    }
    return roff > 0 ? 0 : -1;
}

static int otto_nb_should_intercept_ip(uint32_t ip_h, int *is_vip_out) {
    if ((ip_h & 0xFFFF0000u) == 0x7F580000u) {
        *is_vip_out = 1;
        return 1;
    }
    *is_vip_out = 0;
    const char *priv = getenv("OTTO_NETBRIDGE_PRIVATE_IPS");
    if (!priv || strcmp(priv, "1") != 0) return 0;
    if ((ip_h & 0xFF000000u) == 0x0A000000u) return 1;
    if ((ip_h & 0xFFF00000u) == 0xAC100000u) return 1;
    if ((ip_h & 0xFFFF0000u) == 0xC0A80000u) return 1;
    return 0;
}

#if defined(__APPLE__)
#define REAL_GETADDRINFO getaddrinfo
#define REAL_CONNECT connect
#else
typedef int (*nb_gai_fn)(const char *, const char *, const struct addrinfo *, struct addrinfo **);
typedef int (*nb_connect_fn)(int, const struct sockaddr *, socklen_t);

static int REAL_GETADDRINFO(const char *node, const char *service, const struct addrinfo *hints, struct addrinfo **res) {
    static nb_gai_fn fn = NULL;
    if (!fn) fn = (nb_gai_fn)dlsym(RTLD_NEXT, "getaddrinfo");
    if (!fn) return EAI_SYSTEM;
    return fn(node, service, hints, res);
}

static int REAL_CONNECT(int sockfd, const struct sockaddr *addr, socklen_t addrlen) {
    static nb_connect_fn fn = NULL;
    if (!fn) fn = (nb_connect_fn)dlsym(RTLD_NEXT, "connect");
    if (!fn) {
        errno = ENOSYS;
        return -1;
    }
    return fn(sockfd, addr, addrlen);
}
#endif

#if defined(__APPLE__)
static int otto_getaddrinfo(const char *node, const char *service, const struct addrinfo *hints, struct addrinfo **res)
#else
__attribute__((visibility("default"))) int getaddrinfo(const char *node, const char *service, const struct addrinfo *hints, struct addrinfo **res)
#endif
{
    if (!node || !node[0]) {
        return REAL_GETADDRINFO(node, service, hints, res);
    }
    if (hints && (hints->ai_flags & (AI_PASSIVE | AI_NUMERICHOST)) != 0) {
        return REAL_GETADDRINFO(node, service, hints, res);
    }
    if (strcasecmp(node, "localhost") == 0) {
        return REAL_GETADDRINFO(node, service, hints, res);
    }
    struct in_addr v4tmp;
    struct in6_addr v6tmp;
    if (inet_pton(AF_INET, node, &v4tmp) == 1 || inet_pton(AF_INET6, node, &v6tmp) == 1) {
        return REAL_GETADDRINFO(node, service, hints, res);
    }

    const char *sock_path = getenv("OTTO_NETBRIDGE_SOCK");
    const char *token = getenv("OTTO_NETBRIDGE_TOKEN");
    if (!sock_path || !sock_path[0] || !token || !token[0] || !otto_nb_is_safe_host(node)) {
        return REAL_GETADDRINFO(node, service, hints, res);
    }

    int saved_errno = errno;
    char req[512];
    char resp[128];
    int n = snprintf(req, sizeof(req), "RESOLVE %s %s\n", token, node);
    if (n <= 0 || (size_t)n >= sizeof(req)) {
        errno = saved_errno;
        return REAL_GETADDRINFO(node, service, hints, res);
    }

    if (otto_nb_query(sock_path, req, resp, sizeof(resp)) == 0 && strncmp(resp, "VIP ", 4) == 0) {
        const char *vip = resp + 4;
        if (hints && hints->ai_family == AF_INET6 && (hints->ai_flags & AI_V4MAPPED) == 0) {
            /* Short-circuit IPv6-only lookups immediately so macOS mDNSResponder
               never stalls 5 seconds on `.local` multicast DNS timeouts. */
            errno = saved_errno;
            return EAI_NONAME;
        }
        struct addrinfo nhints;
        if (hints) {
            nhints = *hints;
        } else {
            memset(&nhints, 0, sizeof(nhints));
        }
        if (nhints.ai_family == AF_UNSPEC) {
            nhints.ai_family = AF_INET;
        }
        nhints.ai_flags |= AI_NUMERICHOST;
        errno = saved_errno;
        return REAL_GETADDRINFO(vip, service, &nhints, res);
    }

    errno = saved_errno;
    return REAL_GETADDRINFO(node, service, hints, res);
}

#if defined(__APPLE__)
static int otto_connect(int sockfd, const struct sockaddr *addr, socklen_t addrlen)
#else
__attribute__((visibility("default"))) int connect(int sockfd, const struct sockaddr *addr, socklen_t addrlen)
#endif
{
    if (!addr) {
        return REAL_CONNECT(sockfd, addr, addrlen);
    }

    const char *sock_path = getenv("OTTO_NETBRIDGE_SOCK");
    const char *token = getenv("OTTO_NETBRIDGE_TOKEN");
    if (!sock_path || !sock_path[0] || !token || !token[0]) {
        return REAL_CONNECT(sockfd, addr, addrlen);
    }

    if (addr->sa_family == AF_INET && addrlen >= (socklen_t)sizeof(struct sockaddr_in)) {
        struct sockaddr_in sin;
        memcpy(&sin, addr, sizeof(sin));
        uint32_t ip_h = ntohl(sin.sin_addr.s_addr);
        uint16_t port_h = ntohs(sin.sin_port);
        int is_vip = 0;
        if (port_h != 0 && otto_nb_should_intercept_ip(ip_h, &is_vip)) {
            int saved_errno = errno;
            char ip_buf[INET_ADDRSTRLEN];
            if (inet_ntop(AF_INET, &sin.sin_addr, ip_buf, sizeof(ip_buf)) != NULL) {
                char req[256];
                char resp[128];
                int n = snprintf(req, sizeof(req), "CONNECT %s %s %u\n", token, ip_buf, (unsigned)port_h);
                if (n > 0 && (size_t)n < sizeof(req) && otto_nb_query(sock_path, req, resp, sizeof(resp)) == 0) {
                    if (strncmp(resp, "PORT ", 5) == 0) {
                        unsigned long lport = strtoul(resp + 5, NULL, 10);
                        if (lport >= 1 && lport <= 65535) {
                            sin.sin_addr.s_addr = htonl(0x7F000001u);
                            sin.sin_port = htons((uint16_t)lport);
                            errno = saved_errno;
                            return REAL_CONNECT(sockfd, (const struct sockaddr *)&sin, (socklen_t)sizeof(sin));
                        }
                    } else if (strcmp(resp, "PASSTHROUGH") == 0) {
                        errno = saved_errno;
                        return REAL_CONNECT(sockfd, addr, addrlen);
                    }
                }
            }
            if (is_vip) {
                errno = ECONNREFUSED;
                return -1;
            }
            errno = saved_errno;
        }
        return REAL_CONNECT(sockfd, addr, addrlen);
    }

    if (addr->sa_family == AF_INET6 && addrlen >= (socklen_t)sizeof(struct sockaddr_in6)) {
        struct sockaddr_in6 sin6;
        memcpy(&sin6, addr, sizeof(sin6));
        if (IN6_IS_ADDR_V4MAPPED(&sin6.sin6_addr)) {
            uint32_t ip_n = 0;
            memcpy(&ip_n, &sin6.sin6_addr.s6_addr[12], 4);
            uint32_t ip_h = ntohl(ip_n);
            uint16_t port_h = ntohs(sin6.sin6_port);
            int is_vip = 0;
            if (port_h != 0 && otto_nb_should_intercept_ip(ip_h, &is_vip)) {
                int saved_errno = errno;
                struct in_addr v4;
                v4.s_addr = ip_n;
                char ip_buf[INET_ADDRSTRLEN];
                if (inet_ntop(AF_INET, &v4, ip_buf, sizeof(ip_buf)) != NULL) {
                    char req[256];
                    char resp[128];
                    int n = snprintf(req, sizeof(req), "CONNECT %s %s %u\n", token, ip_buf, (unsigned)port_h);
                    if (n > 0 && (size_t)n < sizeof(req) && otto_nb_query(sock_path, req, resp, sizeof(resp)) == 0) {
                        if (strncmp(resp, "PORT ", 5) == 0) {
                            unsigned long lport = strtoul(resp + 5, NULL, 10);
                            if (lport >= 1 && lport <= 65535) {
                                sin6.sin6_addr.s6_addr[12] = 127;
                                sin6.sin6_addr.s6_addr[13] = 0;
                                sin6.sin6_addr.s6_addr[14] = 0;
                                sin6.sin6_addr.s6_addr[15] = 1;
                                sin6.sin6_port = htons((uint16_t)lport);
                                errno = saved_errno;
                                return REAL_CONNECT(sockfd, (const struct sockaddr *)&sin6, (socklen_t)sizeof(sin6));
                            }
                        } else if (strcmp(resp, "PASSTHROUGH") == 0) {
                            errno = saved_errno;
                            return REAL_CONNECT(sockfd, addr, addrlen);
                        }
                    }
                }
                if (is_vip) {
                    errno = ECONNREFUSED;
                    return -1;
                }
                errno = saved_errno;
            }
        }
        return REAL_CONNECT(sockfd, addr, addrlen);
    }

    return REAL_CONNECT(sockfd, addr, addrlen);
}

#if defined(__APPLE__)
typedef struct {
    const void *replacement;
    const void *replacee;
} otto_dyld_interpose_t;

__attribute__((used, section("__DATA,__interpose")))
static const otto_dyld_interpose_t otto_nb_interposers[] = {
    { (const void *)otto_getaddrinfo, (const void *)getaddrinfo },
    { (const void *)otto_connect, (const void *)connect },
};
#endif
"#;

fn source_digest() -> String {
    let mut hasher = Sha256::new();
    hasher.update(INTERPOSER_C_SRC.as_bytes());
    let full = hex::encode(hasher.finalize());
    full[..16].to_string()
}

/// Compile (once per daemon version) and return the path to the native POSIX
/// interposer shared library (`libotto_netbridge-<hash>.dylib` on macOS, `.so`
/// on Linux). Returns `Ok(None)` if no local C toolchain is installed.
pub async fn ensure_interposer_lib(data_dir: &Path) -> Result<Option<PathBuf>> {
    if !cfg!(any(target_os = "macos", target_os = "linux")) {
        return Ok(None);
    }
    let dir = data_dir.join("ide-netbridge");
    let ext = if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };
    let hash = source_digest();
    let out_path = dir.join(format!("libotto_netbridge-{hash}.{ext}"));
    if tokio::fs::metadata(&out_path).await.is_ok() {
        return Ok(Some(out_path));
    }

    let _guard = BUILD_LOCK.lock().await;
    if tokio::fs::metadata(&out_path).await.is_ok() {
        return Ok(Some(out_path));
    }

    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| Error::Internal(format!("create netbridge dir {}: {e}", dir.display())))?;

    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let src_path = dir.join(format!(".interposer-{nonce}.c"));
    let tmp_out = dir.join(format!(".libotto_netbridge-{nonce}.{ext}"));
    tokio::fs::write(&src_path, INTERPOSER_C_SRC)
        .await
        .map_err(|e| Error::Internal(format!("write interposer source: {e}")))?;

    let compiled = compile_interposer(&src_path, &tmp_out).await;
    let _ = tokio::fs::remove_file(&src_path).await;

    if !compiled {
        let _ = tokio::fs::remove_file(&tmp_out).await;
        tracing::warn!("otto-ide netbridge: C compiler unavailable; falling back to SOCKS5 + env");
        return Ok(None);
    }

    tokio::fs::rename(&tmp_out, &out_path)
        .await
        .map_err(|e| Error::Internal(format!("activate netbridge library: {e}")))?;
    Ok(Some(out_path))
}

async fn compile_interposer(src_path: &Path, out_path: &Path) -> bool {
    if cfg!(target_os = "macos") {
        // Build a universal Mach-O binary first so both native Apple Silicon and
        // Rosetta x86_64 binaries (`telepresence`, older Go/Java CLIs) load cleanly.
        let universal_ok = run_cc(
            "/usr/bin/cc",
            &[
                "-dynamiclib",
                "-O2",
                "-Wall",
                "-Wextra",
                "-arch",
                "arm64",
                "-arch",
                "x86_64",
            ],
            src_path,
            out_path,
        )
        .await;
        if universal_ok {
            return true;
        }
        return run_cc(
            "cc",
            &["-dynamiclib", "-O2", "-Wall", "-Wextra"],
            src_path,
            out_path,
        )
        .await;
    }

    for compiler in ["cc", "gcc", "clang"] {
        if run_cc(
            compiler,
            &["-shared", "-fPIC", "-O2", "-Wall"],
            src_path,
            out_path,
        )
        .await
        {
            return true;
        }
    }
    false
}

async fn run_cc(compiler: &str, flags: &[&str], src: &Path, out: &Path) -> bool {
    let mut cmd = Command::new(compiler);
    cmd.args(flags).arg(src).arg("-o").arg(out);
    if cfg!(target_os = "linux") {
        cmd.arg("-ldl");
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let Ok(child) = cmd.spawn() else {
        return false;
    };
    let Ok(Ok(output)) =
        tokio::time::timeout(std::time::Duration::from_secs(20), child.wait_with_output()).await
    else {
        return false;
    };
    output.status.success()
}

/// Re-export `DYLD_INSERT_LIBRARIES` / `LD_PRELOAD` inside `/bin/sh -c` scripts
/// because macOS SIP strips `DYLD_*` env vars across `/bin/sh` exec boundaries.
pub fn wrap_shell_script_for_interposer(
    script: &str,
    env_map: &BTreeMap<String, String>,
) -> String {
    if !env_map.contains_key("OTTO_NETBRIDGE_DYLIB") {
        return script.to_string();
    }
    if cfg!(target_os = "macos") {
        format!(
            "if [ -n \"${{OTTO_NETBRIDGE_DYLIB:-}}\" ]; then export DYLD_INSERT_LIBRARIES=\"${{OTTO_NETBRIDGE_DYLIB}}\"; fi\n{script}"
        )
    } else if cfg!(target_os = "linux") {
        format!(
            "if [ -n \"${{OTTO_NETBRIDGE_DYLIB:-}}\" ]; then export LD_PRELOAD=\"${{OTTO_NETBRIDGE_DYLIB}}\"; fi\n{script}"
        )
    } else {
        script.to_string()
    }
}
