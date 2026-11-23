//! Instant (`<2ms`, zero-subprocess) Mobile & Hybrid Run Configuration
//! auto-discovery for Flutter, React Native, Expo, Capacitor, Xcode/SwiftPM,
//! and Android Gradle projects.

#![allow(clippy::disallowed_methods)] // synchronous scanner invoked inside spawn_blocking

use std::path::Path;

use crate::build_tools::mobile::{
    detect_android_gradle_stack, detect_flutter_stack, detect_js_mobile_stack, detect_xcode_stack,
    JsMobileKind,
};
use crate::types::{IdeRunConfig, IdeRunnerKind};

use super::discovery::{folder_name, make_auto_config};

pub(super) fn discover_mobile(
    root: &Path,
    proj_name: &str,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) {
    discover_flutter(root, proj_name, root_badge, cwd, out);
    discover_js_mobile(root, root_badge, cwd, out);
    discover_xcode(root, proj_name, root_badge, cwd, out);
    discover_android_gradle(root, proj_name, root_badge, cwd, out);
}

fn discover_flutter(
    root: &Path,
    proj_name: &str,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) {
    let Some(stack) = detect_flutter_stack(root) else {
        return;
    };
    let name = stack.name.unwrap_or_else(|| proj_name.to_string());

    out.push(make_auto_config(
        root_badge,
        IdeRunnerKind::Flutter,
        "flutter-run",
        format!("Flutter Run ({name})"),
        Some("flutter"),
        vec!["run".into()],
        cwd.clone(),
        Some("lib/main.dart".into()),
    ));
    out.push(make_auto_config(
        root_badge,
        IdeRunnerKind::Flutter,
        "flutter-test",
        format!("Flutter Test ({name})"),
        Some("flutter"),
        vec!["test".into()],
        cwd.clone(),
        Some("test".into()),
    ));
}

fn discover_js_mobile(
    root: &Path,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) {
    let Some(stack) = detect_js_mobile_stack(root) else {
        return;
    };
    match stack.kind {
        JsMobileKind::Expo => {
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::ReactNative,
                "expo-start",
                "Expo Start".into(),
                Some("npx"),
                vec!["expo".into(), "start".into()],
                cwd.clone(),
                Some("package.json".into()),
            ));
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::ReactNative,
                "expo-ios",
                "Expo Run iOS".into(),
                Some("npx"),
                vec!["expo".into(), "run:ios".into()],
                cwd.clone(),
                Some("package.json".into()),
            ));
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::ReactNative,
                "expo-android",
                "Expo Run Android".into(),
                Some("npx"),
                vec!["expo".into(), "run:android".into()],
                cwd.clone(),
                Some("package.json".into()),
            ));
        }
        JsMobileKind::ReactNative => {
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::ReactNative,
                "rn-ios",
                "React Native iOS".into(),
                Some("npx"),
                vec!["react-native".into(), "run-ios".into()],
                cwd.clone(),
                Some("package.json".into()),
            ));
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::ReactNative,
                "rn-android",
                "React Native Android".into(),
                Some("npx"),
                vec!["react-native".into(), "run-android".into()],
                cwd.clone(),
                Some("package.json".into()),
            ));
        }
        JsMobileKind::Capacitor => {
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::ReactNative,
                "cap-ios",
                "Capacitor Run iOS".into(),
                Some("npx"),
                vec!["cap".into(), "run".into(), "ios".into()],
                cwd.clone(),
                Some("package.json".into()),
            ));
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::ReactNative,
                "cap-android",
                "Capacitor Run Android".into(),
                Some("npx"),
                vec!["cap".into(), "run".into(), "android".into()],
                cwd.clone(),
                Some("package.json".into()),
            ));
        }
    }
}

fn discover_xcode(
    root: &Path,
    proj_name: &str,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) {
    if let Some(mut stack) = detect_xcode_stack(root) {
        let (flag, bundle_path) = match (&stack.workspace, &stack.xcodeproj) {
            (Some(w), _) => ("-workspace", w.as_path()),
            (None, Some(p)) => ("-project", p.as_path()),
            (None, None) => return,
        };
        let bundle = bundle_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let fallback_scheme = bundle
            .trim_end_matches(".xcworkspace")
            .trim_end_matches(".xcodeproj")
            .to_string();
        if stack.schemes.is_empty() {
            stack.schemes.push(fallback_scheme);
        }
        for scheme in stack.schemes.into_iter().take(2) {
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::Xcode,
                &format!("xcode-build-{scheme}"),
                format!("Xcode Build ({scheme})"),
                Some("xcodebuild"),
                vec![
                    flag.into(),
                    bundle.clone(),
                    "-scheme".into(),
                    scheme.clone(),
                    "-sdk".into(),
                    "iphonesimulator".into(),
                    "build".into(),
                ],
                cwd.clone(),
                Some(bundle.clone()),
            ));
            out.push(make_auto_config(
                root_badge,
                IdeRunnerKind::Xcode,
                &format!("xcode-test-{scheme}"),
                format!("Xcode Test ({scheme})"),
                Some("xcodebuild"),
                vec![
                    flag.into(),
                    bundle.clone(),
                    "-scheme".into(),
                    scheme,
                    "-sdk".into(),
                    "iphonesimulator".into(),
                    "test".into(),
                ],
                cwd.clone(),
                Some(bundle.clone()),
            ));
        }
        return;
    }

    if root.join("Package.swift").is_file() {
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Xcode,
            "swift-build",
            format!("Swift Build ({proj_name})"),
            Some("swift"),
            vec!["build".into()],
            cwd.clone(),
            Some("Package.swift".into()),
        ));
        out.push(make_auto_config(
            root_badge,
            IdeRunnerKind::Xcode,
            "swift-test",
            format!("Swift Test ({proj_name})"),
            Some("swift"),
            vec!["test".into()],
            cwd.clone(),
            Some("Package.swift".into()),
        ));
    }
}

fn discover_android_gradle(
    root: &Path,
    proj_name: &str,
    root_badge: Option<&str>,
    cwd: &Option<String>,
    out: &mut Vec<IdeRunConfig>,
) {
    let android_dir = if detect_android_gradle_stack(root).is_some() {
        root.to_path_buf()
    } else if root.join("android").is_dir()
        && detect_android_gradle_stack(&root.join("android")).is_some()
    {
        root.join("android")
    } else {
        return;
    };

    let gradle_cmd = if android_dir.join("gradlew").is_file() {
        "./gradlew"
    } else {
        "gradle"
    };
    let android_cwd = if android_dir == root {
        cwd.clone()
    } else {
        Some(android_dir.to_string_lossy().into_owned())
    };
    let label = if android_dir == root {
        proj_name.to_string()
    } else {
        folder_name(&android_dir)
    };

    out.push(make_auto_config(
        root_badge,
        IdeRunnerKind::Gradle,
        "android-install-debug",
        format!("Android Install Debug ({label})"),
        Some(gradle_cmd),
        vec!["installDebug".into()],
        android_cwd.clone(),
        None,
    ));
    out.push(make_auto_config(
        root_badge,
        IdeRunnerKind::Gradle,
        "android-assemble-debug",
        format!("Android Assemble Debug ({label})"),
        Some(gradle_cmd),
        vec!["assembleDebug".into()],
        android_cwd.clone(),
        None,
    ));
    out.push(make_auto_config(
        root_badge,
        IdeRunnerKind::Gradle,
        "android-unit-test",
        format!("Android Unit Tests ({label})"),
        Some(gradle_cmd),
        vec!["testDebugUnitTest".into()],
        android_cwd,
        None,
    ));
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::environments::discovery::discover_run_configs;
    use crate::environments::runner_cmd::build_runner_command;
    use crate::types::{IdeRunConfig, IdeRunnerKind, IdeTargetKind, IdeTargetSpec};

    #[test]
    fn discovers_flutter_expo_xcode_and_android_run_configs() {
        let tmp = tempfile::tempdir().unwrap();
        // Flutter project
        let fl = tmp.path().join("fl_app");
        std::fs::create_dir_all(&fl).unwrap();
        std::fs::write(
            fl.join("pubspec.yaml"),
            "name: fl_app\ndependencies:\n  flutter:\n    sdk: flutter\n",
        )
        .unwrap();
        let cfgs = discover_run_configs(&IdeTargetSpec::local(fl.to_string_lossy()), &[]);
        assert!(cfgs
            .iter()
            .any(|c| c.runner == IdeRunnerKind::Flutter && c.args == ["run"]));
        assert!(cfgs
            .iter()
            .any(|c| c.runner == IdeRunnerKind::Flutter && c.args == ["test"]));

        // Xcode workspace with shared scheme
        let ios = tmp.path().join("ios_app");
        std::fs::create_dir_all(ios.join("MyApp.xcworkspace/xcshareddata/xcschemes")).unwrap();
        std::fs::write(
            ios.join("MyApp.xcworkspace/xcshareddata/xcschemes/MyApp.xcscheme"),
            "<Scheme/>",
        )
        .unwrap();
        let ios_cfgs = discover_run_configs(&IdeTargetSpec::local(ios.to_string_lossy()), &[]);
        assert!(ios_cfgs
            .iter()
            .any(|c| c.runner == IdeRunnerKind::Xcode && c.name.contains("MyApp")));

        // Android Gradle project
        let andr = tmp.path().join("android_app");
        std::fs::create_dir_all(andr.join("app/src/main")).unwrap();
        std::fs::write(
            andr.join("app/build.gradle.kts"),
            "plugins { id(\"com.android.application\") }\n",
        )
        .unwrap();
        let andr_cfgs = discover_run_configs(&IdeTargetSpec::local(andr.to_string_lossy()), &[]);
        assert!(andr_cfgs
            .iter()
            .any(|c| c.runner == IdeRunnerKind::Gradle && c.args == ["installDebug"]));
    }

    #[test]
    fn mobile_command_builders_inject_selected_device_flags() {
        let mut env = BTreeMap::new();
        env.insert("OTTO_MOBILE_DEVICE_ID".into(), "emulator-5554".into());
        env.insert("OTTO_SIMULATOR_UDID".into(), "1111-2222".into());

        let fl_cfg = IdeRunConfig {
            runner: IdeRunnerKind::Flutter,
            command: Some("flutter".into()),
            args: vec!["run".into()],
            ..Default::default()
        };
        let (prog, args) = build_runner_command(&fl_cfg, IdeTargetKind::Local, "/tmp", &env);
        assert_eq!(prog, "flutter");
        assert_eq!(args, vec!["run", "-d", "emulator-5554"]);

        let xc_cfg = IdeRunConfig {
            runner: IdeRunnerKind::Xcode,
            command: Some("xcodebuild".into()),
            args: vec!["-scheme".into(), "App".into(), "build".into()],
            ..Default::default()
        };
        let (prog, args) = build_runner_command(&xc_cfg, IdeTargetKind::Local, "/tmp", &env);
        assert_eq!(prog, "xcodebuild");
        assert_eq!(
            args,
            vec!["-scheme", "App", "-destination", "id=1111-2222", "build"]
        );

        let rn_cfg = IdeRunConfig {
            runner: IdeRunnerKind::ReactNative,
            command: Some("npx".into()),
            args: vec!["react-native".into(), "run-ios".into()],
            ..Default::default()
        };
        let (prog, args) = build_runner_command(&rn_cfg, IdeTargetKind::Local, "/tmp", &env);
        assert_eq!(prog, "npx");
        assert_eq!(args, vec!["react-native", "run-ios", "--udid", "1111-2222"]);

        let expo_cfg = IdeRunConfig {
            runner: IdeRunnerKind::ReactNative,
            command: Some("npx".into()),
            args: vec!["expo".into(), "run:ios".into()],
            ..Default::default()
        };
        let (prog, args) = build_runner_command(&expo_cfg, IdeTargetKind::Local, "/tmp", &env);
        assert_eq!(prog, "npx");
        assert_eq!(args, vec!["expo", "run:ios", "--device", "1111-2222"]);

        let cap_cfg = IdeRunConfig {
            runner: IdeRunnerKind::ReactNative,
            command: Some("npx".into()),
            args: vec!["cap".into(), "run".into(), "ios".into()],
            ..Default::default()
        };
        let (prog, args) = build_runner_command(&cap_cfg, IdeTargetKind::Local, "/tmp", &env);
        assert_eq!(prog, "npx");
        assert_eq!(args, vec!["cap", "run", "ios", "--target", "1111-2222"]);
    }

    #[test]
    fn mobile_device_platform_validation_rejects_unknown_platforms() {
        use crate::environments::run_configs::parse_mobile_device_platform;

        assert_eq!(parse_mobile_device_platform(None).unwrap(), None);
        assert_eq!(parse_mobile_device_platform(Some("  ")).unwrap(), None);
        assert_eq!(
            parse_mobile_device_platform(Some("ios"))
                .unwrap()
                .as_deref(),
            Some("ios")
        );
        assert_eq!(
            parse_mobile_device_platform(Some(" android "))
                .unwrap()
                .as_deref(),
            Some("android")
        );
        assert!(parse_mobile_device_platform(Some("web")).is_err());
        assert!(parse_mobile_device_platform(Some("macos")).is_err());
    }

    #[test]
    fn mobile_device_id_validation_rejects_flags_commas_and_oversized_values() {
        use crate::environments::run_configs::is_valid_mobile_device_id;

        assert!(is_valid_mobile_device_id("00008110-001A23456789001E"));
        assert!(is_valid_mobile_device_id("emulator-5554"));
        assert!(is_valid_mobile_device_id("192.168.1.5:5555"));
        assert!(is_valid_mobile_device_id("Pixel_9_API_35"));

        assert!(!is_valid_mobile_device_id(""));
        assert!(!is_valid_mobile_device_id("-flag"));
        assert!(!is_valid_mobile_device_id("1111,platform=macOS"));
        assert!(!is_valid_mobile_device_id("has space"));
        assert!(!is_valid_mobile_device_id("bad;id"));
        assert!(!is_valid_mobile_device_id(&"a".repeat(129)));

        let mut bad_env = BTreeMap::new();
        bad_env.insert("OTTO_MOBILE_DEVICE_ID".into(), "-malicious".into());
        bad_env.insert("OTTO_SIMULATOR_UDID".into(), "1111,platform=macOS".into());

        let fl_cfg = IdeRunConfig {
            runner: IdeRunnerKind::Flutter,
            command: Some("flutter".into()),
            args: vec!["run".into()],
            ..Default::default()
        };
        let (_, fl_args) = build_runner_command(&fl_cfg, IdeTargetKind::Local, "/tmp", &bad_env);
        assert_eq!(fl_args, vec!["run"]);

        let xc_cfg = IdeRunConfig {
            runner: IdeRunnerKind::Xcode,
            command: Some("xcodebuild".into()),
            args: vec!["-scheme".into(), "App".into(), "build".into()],
            ..Default::default()
        };
        let (_, xc_args) = build_runner_command(&xc_cfg, IdeTargetKind::Local, "/tmp", &bad_env);
        assert_eq!(xc_args, vec!["-scheme", "App", "build"]);
    }
}
