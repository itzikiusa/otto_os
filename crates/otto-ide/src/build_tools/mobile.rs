//! Fast (`<15ms`, zero-subprocess) Mobile & Hybrid project detector for
//! [`super::build_tools_model`]: Xcode (`.xcworkspace`/`.xcodeproj`/SwiftPM),
//! Android Gradle (`com.android.application`/`AndroidManifest.xml`), Flutter
//! (`pubspec.yaml`), and React Native / Expo / Capacitor (`package.json`).

#![allow(clippy::disallowed_methods)] // synchronous scanner invoked inside spawn_blocking

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use crate::types::IdeMobileProject;

use super::gradle::shorten_home_path;

pub(super) struct MobileScanResult {
    pub has_xcode: bool,
    pub has_android: bool,
    pub has_flutter: bool,
    pub has_react_native: bool,
    pub has_capacitor: bool,
    pub projects: Vec<IdeMobileProject>,
}

pub(super) fn scan_mobile_roots(
    canon_roots: &[PathBuf],
    home: Option<&Path>,
    is_root_user: bool,
) -> MobileScanResult {
    let mut res = MobileScanResult {
        has_xcode: false,
        has_android: false,
        has_flutter: false,
        has_react_native: false,
        has_capacitor: false,
        projects: Vec::new(),
    };
    let mut seen: HashSet<(String, PathBuf)> = HashSet::new();

    for root in canon_roots {
        let before = res.projects.len();
        scan_dir_for_mobile(root, home, &mut seen, &mut res);
        // If root itself is a multi-repo umbrella, check direct non-symlink subdirs.
        if res.projects.len() == before {
            if let Ok(rd) = fs::read_dir(root) {
                let mut subdirs: Vec<PathBuf> = rd
                    .flatten()
                    .filter(|e| e.file_type().map(|ft| !ft.is_symlink()).unwrap_or(false))
                    .filter_map(|e| {
                        super::confinable_project_dir(&e.path(), canon_roots, is_root_user)
                    })
                    .collect();
                subdirs.sort();
                for sub in subdirs.into_iter().take(24) {
                    scan_dir_for_mobile(&sub, home, &mut seen, &mut res);
                }
            }
        }
    }
    res
}

fn scan_dir_for_mobile(
    dir: &Path,
    home: Option<&Path>,
    seen: &mut HashSet<(String, PathBuf)>,
    out: &mut MobileScanResult,
) {
    // 1. Flutter (`pubspec.yaml` with `sdk: flutter`)
    let mut is_flutter = false;
    if let Some(proj) = detect_flutter(dir, home) {
        if seen.insert((proj.framework.clone(), dir.to_path_buf())) {
            is_flutter = true;
            out.has_flutter = true;
            out.projects.push(proj);
        }
    }

    // 2. React Native / Expo / Capacitor (`package.json`)
    let mut is_js_mobile = false;
    if let Some(proj) = detect_js_mobile(dir, home) {
        if seen.insert((proj.framework.clone(), dir.to_path_buf())) {
            is_js_mobile = true;
            match proj.framework.as_str() {
                "react_native" | "expo" => out.has_react_native = true,
                "capacitor" => out.has_capacitor = true,
                _ => {}
            }
            out.projects.push(proj);
        }
    }

    // 3. Xcode / SwiftPM (check `dir` and, for hybrid apps, `dir/ios`)
    let xcode_dirs: Vec<PathBuf> = if is_flutter || is_js_mobile {
        vec![dir.join("ios")]
    } else {
        vec![dir.to_path_buf(), dir.join("ios")]
    };
    for xd in xcode_dirs {
        if xd.is_dir() {
            if let Some(proj) = detect_xcode_or_swiftpm(&xd, home) {
                if seen.insert((proj.framework.clone(), xd.clone())) {
                    out.has_xcode = true;
                    out.projects.push(proj);
                    break;
                }
            }
        }
    }

    // 4. Android Gradle (check `dir`, `dir/app`, and for hybrid apps `dir/android`)
    let android_dirs: Vec<PathBuf> = if is_flutter || is_js_mobile {
        vec![dir.join("android")]
    } else {
        vec![dir.to_path_buf(), dir.join("android")]
    };
    for ad in android_dirs {
        if ad.is_dir() {
            if let Some(proj) = detect_android_gradle(&ad, home) {
                if seen.insert((proj.framework.clone(), ad.clone())) {
                    out.has_android = true;
                    out.projects.push(proj);
                    break;
                }
            }
        }
    }
}

fn dir_label(dir: &Path) -> String {
    dir.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("mobile-app")
        .to_string()
}

fn read_head(path: &Path) -> Option<String> {
    if fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(true)
    {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    let slice = if bytes.len() > 32 * 1024 {
        &bytes[..32 * 1024]
    } else {
        &bytes[..]
    };
    Some(String::from_utf8_lossy(slice).into_owned())
}

pub(crate) struct FlutterStack {
    pub name: Option<String>,
    pub manifest: PathBuf,
}

pub(crate) fn detect_flutter_stack(dir: &Path) -> Option<FlutterStack> {
    let manifest = dir.join("pubspec.yaml");
    let raw = read_head(&manifest)?;
    // Require `sdk: flutter` or `flutter_test` to avoid false positives on plain Dart packages.
    if !raw.contains("sdk: flutter") && !raw.contains("flutter_test") {
        return None;
    }
    let name = raw.lines().find_map(|l| {
        let t = l.trim();
        t.strip_prefix("name:")
            .map(|v| v.trim().trim_matches(['"', '\'']).to_string())
            .filter(|v| !v.is_empty())
    });
    Some(FlutterStack { name, manifest })
}

fn detect_flutter(dir: &Path, home: Option<&Path>) -> Option<IdeMobileProject> {
    let stack = detect_flutter_stack(dir)?;
    let name = stack.name.unwrap_or_else(|| dir_label(dir));

    let mut platforms = Vec::new();
    for (sub, plat) in [
        ("ios", "ios"),
        ("android", "android"),
        ("web", "web"),
        ("macos", "macos"),
    ] {
        if dir.join(sub).is_dir() {
            platforms.push(plat.to_string());
        }
    }
    if platforms.is_empty() {
        platforms = vec!["ios".into(), "android".into()];
    }

    Some(IdeMobileProject {
        name,
        framework: "flutter".into(),
        platforms,
        root_dir: dir.to_string_lossy().into_owned(),
        location_display: shorten_home_path(dir, home),
        manifest_path: stack.manifest.to_string_lossy().into_owned(),
        app_id: None,
        targets: vec!["lib/main.dart".into()],
        tasks: vec![
            "run".into(),
            "test".into(),
            "build ios --simulator".into(),
            "build apk --debug".into(),
            "pub get".into(),
        ],
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JsMobileKind {
    Expo,
    ReactNative,
    Capacitor,
}

pub(crate) struct JsMobileStack {
    pub kind: JsMobileKind,
    pub name: Option<String>,
    pub manifest: PathBuf,
}

pub(crate) fn detect_js_mobile_stack(dir: &Path) -> Option<JsMobileStack> {
    let manifest = dir.join("package.json");
    let raw = read_head(&manifest)?;
    let val: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let has_dep = |dep: &str| {
        val.get("dependencies").and_then(|d| d.get(dep)).is_some()
            || val
                .get("devDependencies")
                .and_then(|d| d.get(dep))
                .is_some()
    };

    let kind = if has_dep("expo") {
        JsMobileKind::Expo
    } else if has_dep("react-native") {
        JsMobileKind::ReactNative
    } else if has_dep("@capacitor/core")
        || dir.join("capacitor.config.ts").is_file()
        || dir.join("capacitor.config.json").is_file()
    {
        JsMobileKind::Capacitor
    } else {
        return None;
    };

    let name = val
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    Some(JsMobileStack {
        kind,
        name,
        manifest,
    })
}

fn detect_js_mobile(dir: &Path, home: Option<&Path>) -> Option<IdeMobileProject> {
    let stack = detect_js_mobile_stack(dir)?;
    let (framework, tasks) = match stack.kind {
        JsMobileKind::Expo => (
            "expo",
            vec![
                "start".into(),
                "run:ios".into(),
                "run:android".into(),
                "web".into(),
            ],
        ),
        JsMobileKind::ReactNative => (
            "react_native",
            vec![
                "run-ios".into(),
                "run-android".into(),
                "start".into(),
                "test".into(),
            ],
        ),
        JsMobileKind::Capacitor => (
            "capacitor",
            vec!["sync".into(), "run ios".into(), "run android".into()],
        ),
    };

    let name = stack.name.unwrap_or_else(|| dir_label(dir));

    let mut platforms = Vec::new();
    if dir.join("ios").is_dir() || framework == "expo" {
        platforms.push("ios".into());
    }
    if dir.join("android").is_dir() || framework == "expo" {
        platforms.push("android".into());
    }
    if framework == "expo" || framework == "capacitor" {
        platforms.push("web".into());
    }
    if platforms.is_empty() {
        platforms = vec!["ios".into(), "android".into()];
    }

    let app_id = extract_js_mobile_app_id(dir, framework);

    Some(IdeMobileProject {
        name,
        framework: framework.into(),
        platforms,
        root_dir: dir.to_string_lossy().into_owned(),
        location_display: shorten_home_path(dir, home),
        manifest_path: stack.manifest.to_string_lossy().into_owned(),
        app_id,
        targets: Vec::new(),
        tasks,
    })
}

fn extract_js_mobile_app_id(dir: &Path, framework: &str) -> Option<String> {
    if framework == "capacitor" {
        if let Some(raw) = read_head(&dir.join("capacitor.config.json")) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(id) = v.get("appId").and_then(|x| x.as_str()) {
                    return Some(id.to_string());
                }
            }
        }
    }
    if framework == "expo" {
        if let Some(raw) = read_head(&dir.join("app.json")) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(id) = v
                    .pointer("/expo/ios/bundleIdentifier")
                    .or_else(|| v.pointer("/expo/android/package"))
                    .and_then(|x| x.as_str())
                {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

fn collect_xcode_schemes(bundle_dir: &Path) -> Vec<String> {
    let mut schemes = Vec::new();
    let shared_dir = bundle_dir.join("xcshareddata/xcschemes");
    if let Ok(rd) = fs::read_dir(&shared_dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("xcscheme") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    if !schemes.iter().any(|s| s == stem) {
                        schemes.push(stem.to_string());
                    }
                }
            }
        }
    }
    schemes.sort();
    schemes
}

pub(crate) struct XcodeStack {
    pub workspace: Option<PathBuf>,
    pub xcodeproj: Option<PathBuf>,
    pub schemes: Vec<String>,
}

pub(crate) fn detect_xcode_stack(dir: &Path) -> Option<XcodeStack> {
    let mut workspace: Option<PathBuf> = None;
    let mut xcodeproj: Option<PathBuf> = None;
    if let Ok(rd) = fs::read_dir(dir) {
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.ends_with(".xcworkspace") && name != "Pods.xcworkspace" && workspace.is_none() {
                workspace = Some(p);
            } else if name.ends_with(".xcodeproj")
                && name != "Pods.xcodeproj"
                && xcodeproj.is_none()
            {
                xcodeproj = Some(p);
            }
        }
    }
    let bundle = workspace.as_ref().or(xcodeproj.as_ref())?;
    let mut schemes = collect_xcode_schemes(bundle);
    if let Some(ref xp) = xcodeproj {
        for s in collect_xcode_schemes(xp) {
            if !schemes.contains(&s) {
                schemes.push(s);
            }
        }
    }
    Some(XcodeStack {
        workspace,
        xcodeproj,
        schemes,
    })
}

fn detect_xcode_or_swiftpm(dir: &Path, home: Option<&Path>) -> Option<IdeMobileProject> {
    if let Some(mut stack) = detect_xcode_stack(dir) {
        let bundle = stack
            .workspace
            .as_ref()
            .or(stack.xcodeproj.as_ref())
            .expect("XcodeStack always has a workspace or xcodeproj");
        let name = bundle
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("iOS App")
            .to_string();
        if stack.schemes.is_empty() {
            stack.schemes.push(name.clone());
        }
        return Some(IdeMobileProject {
            name,
            framework: "xcode".into(),
            platforms: vec!["ios".into()],
            root_dir: dir.to_string_lossy().into_owned(),
            location_display: shorten_home_path(dir, home),
            manifest_path: bundle.to_string_lossy().into_owned(),
            app_id: None,
            targets: stack.schemes,
            tasks: vec!["build".into(), "test".into(), "clean".into()],
        });
    }

    let pkg_swift = dir.join("Package.swift");
    if pkg_swift.is_file() {
        let raw = read_head(&pkg_swift).unwrap_or_default();
        static RE_PKG_NAME: OnceLock<Regex> = OnceLock::new();
        let re = RE_PKG_NAME.get_or_init(|| Regex::new(r#"name\s*:\s*"([^"]+)""#).unwrap());
        let name = re
            .captures(&raw)
            .map(|c| c[1].to_string())
            .unwrap_or_else(|| dir_label(dir));
        return Some(IdeMobileProject {
            name,
            framework: "swiftpm".into(),
            platforms: vec!["ios".into(), "macos".into()],
            root_dir: dir.to_string_lossy().into_owned(),
            location_display: shorten_home_path(dir, home),
            manifest_path: pkg_swift.to_string_lossy().into_owned(),
            app_id: None,
            targets: Vec::new(),
            tasks: vec!["build".into(), "test".into()],
        });
    }

    None
}

pub(crate) fn is_android_gradle_content(raw: &str) -> bool {
    raw.contains("com.android.application")
        || raw.contains("com.android.library")
        || raw.contains("com.android.dynamic-feature")
        || raw.contains("androidApplication")
}

pub(crate) struct AndroidGradleStack {
    pub manifest: PathBuf,
    pub raw_gradle: String,
}

pub(crate) fn detect_android_gradle_stack(dir: &Path) -> Option<AndroidGradleStack> {
    let candidates = [
        dir.join("app/build.gradle.kts"),
        dir.join("app/build.gradle"),
        dir.join("build.gradle.kts"),
        dir.join("build.gradle"),
    ];
    let mut matched_manifest: Option<PathBuf> = None;
    let mut matched_raw = String::new();
    for cand in &candidates {
        if let Some(raw) = read_head(cand) {
            if is_android_gradle_content(&raw) {
                matched_manifest = Some(cand.clone());
                matched_raw = raw;
                break;
            }
        }
    }
    let has_android_manifest = dir.join("app/src/main/AndroidManifest.xml").is_file()
        || dir.join("src/main/AndroidManifest.xml").is_file();
    if matched_manifest.is_none() && !has_android_manifest {
        return None;
    }
    let manifest = matched_manifest.unwrap_or_else(|| {
        if dir.join("app/src/main/AndroidManifest.xml").is_file() {
            dir.join("app/src/main/AndroidManifest.xml")
        } else {
            dir.join("src/main/AndroidManifest.xml")
        }
    });
    Some(AndroidGradleStack {
        manifest,
        raw_gradle: matched_raw,
    })
}

fn detect_android_gradle(dir: &Path, home: Option<&Path>) -> Option<IdeMobileProject> {
    let stack = detect_android_gradle_stack(dir)?;
    let manifest = stack.manifest;
    let matched_raw = stack.raw_gradle;

    static RE_APP_ID: OnceLock<Regex> = OnceLock::new();
    let app_id_re = RE_APP_ID.get_or_init(|| {
        Regex::new(r#"(?:applicationId|namespace)\s*=?\s*['"]([^'"]+)['"]"#).unwrap()
    });
    let app_id = app_id_re
        .captures(&matched_raw)
        .map(|c| c[1].trim().to_string())
        .filter(|s| !s.is_empty());

    let name = dir_label(dir);
    Some(IdeMobileProject {
        name,
        framework: "android".into(),
        platforms: vec!["android".into()],
        root_dir: dir.to_string_lossy().into_owned(),
        location_display: shorten_home_path(dir, home),
        manifest_path: manifest.to_string_lossy().into_owned(),
        app_id,
        targets: vec!["app".into()],
        tasks: vec![
            "assembleDebug".into(),
            "installDebug".into(),
            "testDebugUnitTest".into(),
            "connectedDebugAndroidTest".into(),
            "lintDebug".into(),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_tools::build_tools_model;
    use crate::types::IdeBuildToolsModelReq;

    #[test]
    fn detects_flutter_xcode_android_and_expo_projects() {
        let tmp = tempfile::tempdir().unwrap();
        let flutter_dir = tmp.path().join("my_flutter");
        fs::create_dir_all(flutter_dir.join("ios/Runner.xcworkspace/xcshareddata/xcschemes"))
            .unwrap();
        fs::create_dir_all(flutter_dir.join("android/app/src/main")).unwrap();
        fs::create_dir_all(flutter_dir.join("web")).unwrap();
        fs::write(
            flutter_dir.join("pubspec.yaml"),
            "name: demo_flutter\ndependencies:\n  flutter:\n    sdk: flutter\n",
        )
        .unwrap();
        fs::write(
            flutter_dir.join("ios/Runner.xcworkspace/xcshareddata/xcschemes/Runner.xcscheme"),
            "<Scheme/>",
        )
        .unwrap();
        fs::write(
            flutter_dir.join("android/app/build.gradle.kts"),
            "plugins { id(\"com.android.application\") }\nandroid { applicationId = \"com.example.demo\" }\n",
        )
        .unwrap();

        let resp = build_tools_model(
            &IdeBuildToolsModelReq {
                target: None,
                roots: vec![flutter_dir.to_string_lossy().into_owned()],
            },
            false,
        );
        assert!(resp.has_flutter);
        assert!(resp.has_xcode);
        assert!(resp.has_android);
        assert_eq!(resp.mobile_projects.len(), 3);
        let fl = resp
            .mobile_projects
            .iter()
            .find(|p| p.framework == "flutter")
            .unwrap();
        assert_eq!(fl.name, "demo_flutter");
        assert!(fl.platforms.contains(&"ios".to_string()));
        assert!(fl.platforms.contains(&"android".to_string()));
        assert!(fl.platforms.contains(&"web".to_string()));
        let xc = resp
            .mobile_projects
            .iter()
            .find(|p| p.framework == "xcode")
            .unwrap();
        assert_eq!(xc.targets, vec!["Runner"]);
        let an = resp
            .mobile_projects
            .iter()
            .find(|p| p.framework == "android")
            .unwrap();
        assert_eq!(an.app_id.as_deref(), Some("com.example.demo"));

        // Expo + Capacitor detection
        let expo_dir = tmp.path().join("my_expo");
        fs::create_dir_all(&expo_dir).unwrap();
        fs::write(
            expo_dir.join("package.json"),
            r#"{"name":"expo-shop","dependencies":{"expo":"^52.0.0","react-native":"0.76.0"}}"#,
        )
        .unwrap();
        fs::write(
            expo_dir.join("app.json"),
            r#"{"expo":{"ios":{"bundleIdentifier":"com.shop.app"}}}"#,
        )
        .unwrap();
        let expo_resp = build_tools_model(
            &IdeBuildToolsModelReq {
                target: None,
                roots: vec![expo_dir.to_string_lossy().into_owned()],
            },
            false,
        );
        assert!(expo_resp.has_react_native);
        assert_eq!(expo_resp.mobile_projects[0].framework, "expo");
        assert_eq!(
            expo_resp.mobile_projects[0].app_id.as_deref(),
            Some("com.shop.app")
        );
    }
}
