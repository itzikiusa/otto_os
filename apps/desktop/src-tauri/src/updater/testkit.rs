//! Test fixtures for the updater: a loopback HTTP server, raw tar archives,
//! and bundles signed by TEST identities that live only in a temp keychain
//! file (deleted on drop — never the login keychain, never the search list).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use reqwest::Url;

use super::stage::{APP_ID, BUNDLE};

static NEXT: AtomicUsize = AtomicUsize::new(0);

pub fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "otto-upd-{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A tiny native program that sleeps for 60 s: the stand-in app process for
/// finisher tests. It lives OUTSIDE any bundle and is exec'd directly, never
/// through LaunchServices (a copied system binary is killed at launch).
pub fn waiter() -> &'static str {
    static EXE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    EXE.get_or_init(|| {
        let dir = temp_dir("waiter");
        std::fs::write(
            dir.join("w.c"),
            "#include <unistd.h>\nint main(void){ sleep(60); return 0; }\n",
        )
        .unwrap();
        run(Command::new("cc")
            .current_dir(&dir)
            .args(["-o", "waiter", "w.c"]));
        dir.join("waiter").to_string_lossy().into_owned()
    })
}

/// (path, status, body, Location header)
pub type Route = (&'static str, u16, Vec<u8>, Option<String>);

/// A tiny HTTP/1.1 server on 127.0.0.1 (one thread, `Connection: close`).
/// Routes can be replaced while it runs.
pub struct Server {
    pub port: u16,
    pub routes: Arc<Mutex<Vec<Route>>>,
}

impl Server {
    pub fn start(routes: Vec<Route>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes = Arc::new(Mutex::new(routes));
        let shared = routes.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut line = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                let found = shared.lock().unwrap().iter().find(|r| r.0 == path).cloned();
                let (status, body, location) = match found {
                    Some((_, s, b, l)) => (s, b, l),
                    None => (404, b"not found".to_vec(), None),
                };
                let mut head = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                if let Some(l) = location {
                    head.push_str(&format!("Location: {l}\r\n"));
                }
                head.push_str("\r\n");
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        Server { port, routes }
    }

    pub fn url(&self, path: &str) -> Url {
        format!("http://127.0.0.1:{}{path}", self.port)
            .parse()
            .unwrap()
    }

    pub fn set(&self, path: &'static str, status: u16, body: Vec<u8>) {
        let mut r = self.routes.lock().unwrap();
        r.retain(|x| x.0 != path);
        r.push((path, status, body, None));
    }
}

/// One raw tar entry (written byte by byte, so hostile names survive).
pub struct TarEntry {
    name: String,
    kind: u8,
    data: Vec<u8>,
    link: String,
}

impl TarEntry {
    pub fn file(name: &str, data: &[u8]) -> TarEntry {
        TarEntry {
            name: name.into(),
            kind: b'0',
            data: data.to_vec(),
            link: String::new(),
        }
    }
    pub fn dir(name: &str) -> TarEntry {
        TarEntry {
            name: format!("{name}/"),
            kind: b'5',
            data: vec![],
            link: String::new(),
        }
    }
    pub fn link(name: &str, target: &str) -> TarEntry {
        TarEntry {
            name: name.into(),
            kind: b'2',
            data: vec![],
            link: target.into(),
        }
    }
    pub fn hard(name: &str, target: &str) -> TarEntry {
        TarEntry {
            name: name.into(),
            kind: b'1',
            data: vec![],
            link: target.into(),
        }
    }
}

pub fn tar_gz(entries: &[TarEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in entries {
        let mut h = tar::Header::new_ustar();
        h.as_old_mut().name[..e.name.len()].copy_from_slice(e.name.as_bytes());
        h.as_old_mut().linkname[..e.link.len()].copy_from_slice(e.link.as_bytes());
        h.set_entry_type(tar::EntryType::new(e.kind));
        h.set_mode(if e.kind == b'5' { 0o755 } else { 0o644 });
        h.set_size(e.data.len() as u64);
        h.set_cksum();
        out.extend_from_slice(h.as_bytes());
        out.extend_from_slice(&e.data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&out).unwrap();
    gz.finish().unwrap()
}

fn run(cmd: &mut Command) -> String {
    let out = cmd.output().expect("spawn");
    assert!(
        out.status.success(),
        "{cmd:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// stdout + stderr of a diagnostic command (never fails).
fn probe(args: &[&str]) -> String {
    match Command::new(args[0]).args(&args[1..]).output() {
        Ok(o) => format!(
            "$ {}\n{}{}",
            args.join(" "),
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(e) => format!("$ {}: {e}\n", args.join(" ")),
    }
}

/// The macOS OpenSSL (LibreSSL), the same on every Mac and runner — never
/// whichever `openssl` PATH finds first.
const OPENSSL: &str = "/usr/bin/openssl";

/// Serializes the CI-only search-list fallback in [`Kit::sign`].
static SEARCH_LIST: Mutex<()> = Mutex::new(());

/// TEST code-signing identities in a temp keychain file + bundles signed by
/// them, laid out like the real app (bundle + `ottod` + optional `kiri`).
/// The keychain is passed explicitly to every command; it is never added to
/// the keychain search list (except the loud, CI-only fallback in `sign`).
pub struct Kit {
    pub dir: PathBuf,
    keychain: PathBuf,
    /// CN → leaf SHA-1: codesign signs by hash, not by a name match.
    ids: Mutex<std::collections::HashMap<String, String>>,
}

const KC_PW: &str = "otto-updater-test-only";

impl Kit {
    pub fn new(tag: &str) -> Kit {
        let dir = temp_dir(&format!("kit-{tag}"));
        let keychain = dir.join("TEST.keychain-db");
        let kc = keychain.to_str().unwrap();
        run(Command::new("/usr/bin/security").args(["create-keychain", "-p", KC_PW, kc]));
        run(Command::new("/usr/bin/security").args(["set-keychain-settings", kc]));
        run(Command::new("/usr/bin/security").args(["unlock-keychain", "-p", KC_PW, kc]));
        Kit {
            dir,
            keychain,
            ids: Mutex::new(Default::default()),
        }
    }

    /// Everything that explains a signing failure on a runner.
    fn diagnostics(&self) -> String {
        let kc = self.keychain.to_str().unwrap();
        [
            probe(&["/usr/bin/sw_vers", "-productVersion"]),
            probe(&[OPENSSL, "version"]),
            probe(&[
                "/usr/bin/security",
                "find-identity",
                "-p",
                "codesigning",
                kc,
            ]),
            probe(&["/usr/bin/security", "show-keychain-info", kc]),
            probe(&["/usr/bin/security", "list-keychains", "-d", "user"]),
            probe(&["/usr/bin/security", "default-keychain", "-d", "user"]),
        ]
        .concat()
    }

    /// A self-signed code-signing identity; returns its leaf SHA-1 (hex).
    pub fn identity(&self, cn: &str) -> String {
        let d = self.dir.join(cn.replace(' ', "_"));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("cert.conf"),
            format!("[ req ]\ndistinguished_name = dn\nx509_extensions = ext\nprompt = no\n[ dn ]\nCN = {cn}\n[ ext ]\nkeyUsage = critical, digitalSignature\nextendedKeyUsage = critical, codeSigning\nbasicConstraints = critical, CA:false\n"),
        )
        .unwrap();
        run(Command::new(OPENSSL).current_dir(&d).args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-keyout",
            "key.pem",
            "-out",
            "cert.pem",
            "-days",
            "2",
            "-nodes",
            "-config",
            "cert.conf",
        ]));
        // SHA1/3DES PKCS#12: what every macOS `security import` reads
        // (packaging/make-cert.sh --export uses the same).
        run(Command::new(OPENSSL).current_dir(&d).args([
            "pkcs12",
            "-export",
            "-inkey",
            "key.pem",
            "-in",
            "cert.pem",
            "-out",
            "cert.p12",
            "-passout",
            "pass:p12",
            "-keypbe",
            "PBE-SHA1-3DES",
            "-certpbe",
            "PBE-SHA1-3DES",
            "-macalg",
            "sha1",
        ]));
        let kc = self.keychain.to_str().unwrap();
        run(Command::new("/usr/bin/security").args([
            "import",
            d.join("cert.p12").to_str().unwrap(),
            "-k",
            kc,
            "-P",
            "p12",
            "-T",
            "/usr/bin/codesign",
            "-f",
            "pkcs12",
        ]));
        run(Command::new("/usr/bin/security").args([
            "set-key-partition-list",
            "-S",
            "apple-tool:,apple:,codesign:",
            "-s",
            "-k",
            KC_PW,
            kc,
        ]));
        let der = run(Command::new("/bin/sh").current_dir(&d).args([
            "-c",
            &format!("{OPENSSL} x509 -in cert.pem -outform der | /usr/bin/shasum -a 1"),
        ]));
        let hash = der.split_whitespace().next().unwrap().to_string();
        // The identity (cert + private key) must be in the keychain file now.
        let found = probe(&[
            "/usr/bin/security",
            "find-identity",
            "-p",
            "codesigning",
            kc,
        ]);
        assert!(
            found.to_uppercase().contains(&hash.to_uppercase()),
            "testkit: the TEST identity {cn} ({hash}) is not in its keychain after import\n{}",
            self.diagnostics()
        );
        self.ids
            .lock()
            .unwrap()
            .insert(cn.to_string(), hash.clone());
        hash
    }

    fn sign(&self, path: &Path, cn: &str, deep: bool) {
        let hash = self
            .ids
            .lock()
            .unwrap()
            .get(cn)
            .cloned()
            .expect("identity() first");
        let attempt = || {
            let mut cmd = Command::new("/usr/bin/codesign");
            cmd.args([
                "--force",
                "--options",
                "runtime",
                "--timestamp=none",
                "--keychain",
            ])
            .arg(&self.keychain)
            .args(["-s", &hash]);
            if deep {
                cmd.arg("--deep");
            }
            cmd.arg(path).output().expect("spawn codesign")
        };
        let mut out = attempt();
        let stderr = |o: &std::process::Output| String::from_utf8_lossy(&o.stderr).into_owned();
        // Some codesign builds only find an identity in a keychain that is on
        // the search list, even with --keychain. On CI only (an ephemeral
        // runner), retry with the temp keychain added to it, then restore
        // the exact previous list. A developer's Mac is never touched.
        if !out.status.success()
            && stderr(&out).contains("no identity found")
            && std::env::var_os("GITHUB_ACTIONS").is_some()
        {
            let _one = SEARCH_LIST.lock().unwrap_or_else(|p| p.into_inner());
            let listed =
                run(Command::new("/usr/bin/security").args(["list-keychains", "-d", "user"]));
            let before: Vec<String> = listed
                .lines()
                .map(|l| l.trim().trim_matches('"').to_string())
                .filter(|l| !l.is_empty())
                .collect();
            eprintln!(
                "testkit: codesign found no identity in {} — CI only: retrying with it on the user search list (restored after)",
                self.keychain.display()
            );
            let mut with_kc = before.clone();
            with_kc.push(self.keychain.to_string_lossy().into_owned());
            run(Command::new("/usr/bin/security")
                .args(["list-keychains", "-d", "user", "-s"])
                .args(&with_kc));
            out = attempt();
            run(Command::new("/usr/bin/security")
                .args(["list-keychains", "-d", "user", "-s"])
                .args(&before));
        }
        assert!(
            out.status.success(),
            "testkit: codesign -s {hash} ({cn}) {} failed: {}\n{}",
            path.display(),
            stderr(&out),
            self.diagnostics()
        );
    }

    /// An unsigned, INERT test bundle `<dir>/<BUNDLE>` at `version`: never
    /// Otto's name or bundle id, package type `BNDL` (not an app),
    /// background-only, never launched. With `binaries`, `Contents/MacOS`
    /// holds `otto-desktop` + `ottod` (+ `kiri`) copies of /usr/bin/true for
    /// codesign to seal; without, there is nothing executable in it at all.
    pub fn plain_bundle(dir: &Path, version: &str, binaries: bool, with_kiri: bool) -> PathBuf {
        let app = dir.join(BUNDLE);
        let _ = std::fs::remove_dir_all(&app);
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        std::fs::write(app.join("Contents/Resources/notes.txt"), version).unwrap();
        let exe = if binaries {
            "<key>CFBundleExecutable</key><string>otto-desktop</string>\n"
        } else {
            ""
        };
        std::fs::write(
            app.join("Contents/Info.plist"),
            format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>{APP_ID}</string>
{exe}<key>CFBundleName</key><string>OttoUpdateTest</string>
<key>CFBundlePackageType</key><string>BNDL</string>
<key>LSBackgroundOnly</key><true/>
<key>LSUIElement</key><true/>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleVersion</key><string>{version}</string>
</dict></plist>
"#),
        )
        .unwrap();
        if binaries {
            let mut bins = vec!["otto-desktop", "ottod"];
            if with_kiri {
                bins.push("kiri");
            }
            for b in bins {
                std::fs::copy("/usr/bin/true", app.join("Contents/MacOS").join(b)).unwrap();
            }
        }
        app
    }

    /// A bundle signed like packaging/sign.sh: sidecars first, then the
    /// bundle with `--deep`.
    pub fn bundle(&self, tag: &str, version: &str, cn: &str, with_kiri: bool) -> PathBuf {
        self.bundle_mixed_inner(tag, version, cn, cn, with_kiri)
    }

    /// Like [`Self::bundle`] but `ottod` is signed by `sidecar_cn` and the
    /// bundle seal (without --deep, so the sidecar keeps its signature) by `cn`.
    pub fn bundle_mixed(&self, tag: &str, version: &str, cn: &str, sidecar_cn: &str) -> PathBuf {
        self.bundle_mixed_inner(tag, version, cn, sidecar_cn, false)
    }

    fn bundle_mixed_inner(
        &self,
        tag: &str,
        version: &str,
        cn: &str,
        sidecar_cn: &str,
        with_kiri: bool,
    ) -> PathBuf {
        let app = Self::plain_bundle(&self.dir.join(tag), version, true, with_kiri);
        let macos = app.join("Contents/MacOS");
        self.sign(&macos.join("ottod"), sidecar_cn, false);
        if with_kiri {
            self.sign(&macos.join("kiri"), cn, false);
        }
        self.sign(&app, cn, cn == sidecar_cn);
        app
    }

    /// The release-style archive of `app` (its parent holds only it).
    pub fn archive(&self, app: &Path) -> Vec<u8> {
        let mut child = Command::new("/usr/bin/tar")
            .env("COPYFILE_DISABLE", "1")
            .args(["--no-mac-metadata", "--no-xattrs", "-czf", "-", "-C"])
            .arg(app.parent().unwrap())
            .arg(BUNDLE)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut buf = Vec::new();
        child.stdout.take().unwrap().read_to_end(&mut buf).unwrap();
        assert!(child.wait().unwrap().success());
        buf
    }
}

impl Drop for Kit {
    fn drop(&mut self) {
        let _ = Command::new("/usr/bin/security")
            .arg("delete-keychain")
            .arg(&self.keychain)
            .output();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A TEST minisign key pair and a signer that writes Tauri's `.sig` format
/// (base64 of the minisign text, trusted comment binding `version:`).
pub struct Minisign {
    kp: minisign::KeyPair,
}

impl Minisign {
    pub fn new() -> Minisign {
        Minisign {
            kp: minisign::KeyPair::generate_unencrypted_keypair().unwrap(),
        }
    }
    /// `plugins.updater.pubkey`: base64 of the public key box.
    pub fn pubkey(&self) -> String {
        use base64_engine::encode;
        encode(self.kp.pk.to_box().unwrap().to_string().as_bytes())
    }
    pub fn sign(&self, data: &[u8], version: Option<&str>) -> String {
        use base64_engine::encode;
        let trusted = match version {
            Some(v) => format!("timestamp:1\tfile:update.tar.gz\tversion:{v}"),
            None => "timestamp:1\tfile:update.tar.gz".to_string(),
        };
        let sig = minisign::sign(
            Some(&self.kp.pk),
            &self.kp.sk,
            data,
            Some(&trusted),
            Some("signature from tauri secret key"),
        )
        .unwrap();
        encode(sig.to_string().as_bytes())
    }
}

/// Standard base64 without another dependency.
mod base64_engine {
    pub fn encode(input: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in input.chunks(3) {
            let n = (c[0] as u32) << 16
                | (*c.get(1).unwrap_or(&0) as u32) << 8
                | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= c.len() {
                    out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }
}

/// No fixture may pass for Otto (a test bundle named like the app, opened
/// through LaunchServices, once showed the owner "Otto is damaged"): not its
/// name, not its bundle id, never an app or a launchable bundle — and no
/// updater test source spells either.
#[test]
fn no_test_fixture_can_pass_for_otto() {
    let (name, id) = (concat!("Otto", ".app"), concat!("com.otto", ".app"));
    assert_ne!(BUNDLE, name);
    assert_ne!(APP_ID, id);
    let dir = temp_dir("guard");
    for binaries in [false, true] {
        let b = Kit::plain_bundle(&dir, "1.0.0", binaries, true);
        assert_eq!(b.file_name().unwrap().to_str().unwrap(), BUNDLE);
        let plist = std::fs::read_to_string(b.join("Contents/Info.plist")).unwrap();
        assert!(plist.contains(&format!("<string>{APP_ID}</string>")) && !plist.contains(id));
        assert!(
            plist.contains("<key>CFBundlePackageType</key><string>BNDL</string>"),
            "never an app"
        );
        assert!(plist.contains("<key>LSBackgroundOnly</key><true/>"));
        assert_eq!(
            plist.contains("CFBundleExecutable"),
            binaries,
            "inert unless codesign needs one"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    let sources = [
        ("testkit.rs", include_str!("testkit.rs")),
        ("fetch.rs", include_str!("fetch.rs")),
        ("stage.rs", include_str!("stage.rs")),
        ("finish.rs", include_str!("finish.rs")),
        ("mod.rs", include_str!("mod.rs")),
    ];
    for (file, src) in sources {
        let tests = match file {
            "testkit.rs" => src,
            _ => &src[src.find("#[cfg(test)]\nmod tests").expect(file)..],
        };
        assert!(
            !tests.contains(name) && !tests.contains(id),
            "{file}'s tests name Otto's real bundle or id"
        );
    }
}
