use super::*;

#[test]
fn single_line_empty_package_json_deps_never_misparse_scripts() {
    let pkg = r#"{
  "name": "demo",
  "dependencies": {},
  "devDependencies": {},
  "scripts": {
    "test": "vitest",
    "build": "tsc"
  }
}
"#;
    let items = parse_package_json(pkg);
    assert!(
        items.is_empty(),
        "empty dependencies/devDependencies must not leak into scripts: {items:?}"
    );
}

#[test]
fn go_mod_single_line_require_parens_preserve_replace_blocks_and_token_boundaries() {
    let go_mod = "module example.com/app\n\ngo 1.22\n\nrequire ()\n\nreplace (\n\texample.com/old => ../local_old\n)\n\nrequire (example.com/inline v1.2.0)\nrequire (\n\tgithub.com/foo/bar v1.0.0\n\tgithub.com/foo/bar-baz v2.0.0\n)\n";
    let items = parse_go_mod(go_mod);
    let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "example.com/inline",
            "github.com/foo/bar",
            "github.com/foo/bar-baz"
        ]
    );

    let bumped =
        bump_go_mod_in_place(go_mod, "github.com/foo/bar", "v1.1.0").expect("bump exact module");
    assert!(bumped.contains("\tgithub.com/foo/bar v1.1.0\n"));
    assert!(bumped.contains("\tgithub.com/foo/bar-baz v2.0.0\n"));

    let merged = merge_go_mod_requires(go_mod);
    assert!(
        merged.contains("replace (\n\texample.com/old => ../local_old\n)"),
        "merge_go_mod_requires must preserve replace block after require (): {merged}"
    );
    assert!(merged.contains("\texample.com/inline v1.2.0"));
}

#[test]
fn versionless_bom_pom_dependency_inserts_version_tag_after_artifact_id() {
    let pom = r#"<project>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.springframework.boot</groupId>
        <artifactId>spring-boot-dependencies</artifactId>
        <version>3.2.0</version>
        <type>pom</type>
        <scope>import</scope>
      </dependency>
    </dependencies>
  </dependencyManagement>
  <dependencies>
    <dependency>
      <groupId>org.springframework.boot</groupId>
      <artifactId>spring-boot-starter-web</artifactId>
    </dependency>
  </dependencies>
</project>
"#;
    let updated = bump_manifest_version_in_place(
        Path::new("/tmp/nonexistent/pom.xml"),
        "pom.xml",
        pom,
        "org.springframework.boot:spring-boot-starter-web",
        "3.3.1",
    )
    .expect("insert version tag");
    assert!(
        updated.contains(
            "      <artifactId>spring-boot-starter-web</artifactId>\n      <version>3.3.1</version>\n"
        ),
        "expected clean <version> insertion after <artifactId>, got:\n{updated}"
    );
    assert!(!updated.contains("3.3.1    <dependency>"));
}

#[tokio::test]
async fn deps_action_rejects_flag_injection_and_whitespace_specs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let go_mod_path = root.join("go.mod");
    fs::write(&go_mod_path, "module example.com/app\n\ngo 1.22\n").unwrap();
    let history = LocalHistoryStore::new(root.join(".history"));

    for (action, bad_mod, bad_ver) in [
        ("why", Some("-toolexec=/bin/sh"), None),
        ("update_one", Some("-modfile=evil.mod"), Some("v1.0.0")),
        ("update_one", Some("example.com/pkg"), Some("-u=evil")),
        ("update_one", Some("example.com/pkg extra"), Some("v1.0.0")),
    ] {
        let req = IdeDepsActionReq {
            target: None,
            path: go_mod_path.to_string_lossy().into_owned(),
            action: action.into(),
            module: bad_mod.map(str::to_string),
            version: bad_ver.map(str::to_string),
        };
        let err = run_deps_action(&root, &go_mod_path, &req, &BTreeMap::new(), &history)
            .await
            .expect_err("flag or whitespace injection must be rejected");
        assert!(
            matches!(err, Error::Invalid(_)),
            "expected Error::Invalid, got {err:?}"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn go_mod_update_one_surfaces_go_get_non_zero_exit_and_scrubs_daemon_env() {
    use std::os::unix::fs::PermissionsExt;

    if otto_core::e2e::isolated() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let bin_dir = root.join("fake-bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let fake_go = bin_dir.join("go");
    fs::write(
        &fake_go,
        "#!/bin/sh\necho \"go: module example.com/missing@latest: no matching versions\" >&2\nexit 1\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&fake_go).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&fake_go, perms).unwrap();

    let go_mod_path = root.join("go.mod");
    fs::write(
        &go_mod_path,
        "module example.com/app\n\ngo 1.22\n\nrequire example.com/missing v0.1.0\n",
    )
    .unwrap();
    let history = LocalHistoryStore::new(root.join(".history"));
    let mut env_map = BTreeMap::new();
    env_map.insert(
        "PATH".into(),
        format!("{}:/usr/bin:/bin", bin_dir.display()),
    );

    let req = IdeDepsActionReq {
        target: None,
        path: go_mod_path.to_string_lossy().into_owned(),
        action: "update_one".into(),
        module: Some("example.com/missing".into()),
        version: Some("latest".into()),
    };
    let resp = run_deps_action(&root, &go_mod_path, &req, &env_map, &history)
        .await
        .expect("run_deps_action should return structured failure response");
    assert!(!resp.ok, "non-zero go get exit must return ok = false");
    assert!(
        resp.output.contains("no matching versions"),
        "expected go get stderr in output, got: {}",
        resp.output
    );
}

#[cfg(unix)]
#[tokio::test]
async fn go_mod_tidy_passes_ssh_auth_sock_only_for_root_actor() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let bin_dir = root.join("fake-bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let fake_go = bin_dir.join("go");
    fs::write(
        &fake_go,
        "#!/bin/sh\necho \"SOCK=${SSH_AUTH_SOCK:-none}\"\nexit 0\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&fake_go).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&fake_go, perms).unwrap();

    let go_mod_path = root.join("go.mod");
    fs::write(&go_mod_path, "module example.com/app\n\ngo 1.22\n").unwrap();
    let history = LocalHistoryStore::new(root.join(".history"));
    let mut env_map = BTreeMap::new();
    env_map.insert(
        "PATH".into(),
        format!("{}:/usr/bin:/bin", bin_dir.display()),
    );
    let req = IdeDepsActionReq {
        target: None,
        path: go_mod_path.to_string_lossy().into_owned(),
        action: "tidy".into(),
        module: None,
        version: None,
    };

    let non_root_resp =
        super::run_deps_action_as(&root, &go_mod_path, &req, &env_map, &history, false)
            .await
            .unwrap();
    assert!(non_root_resp.ok);
    assert_eq!(non_root_resp.output.trim(), "SOCK=none");

    if let Ok(daemon_sock) = std::env::var("SSH_AUTH_SOCK") {
        if !daemon_sock.is_empty() {
            let root_resp =
                super::run_deps_action_as(&root, &go_mod_path, &req, &env_map, &history, true)
                    .await
                    .unwrap();
            assert!(root_resp.ok);
            assert_eq!(root_resp.output.trim(), format!("SOCK={daemon_sock}"));
        }
    }
}

#[test]
fn merge_go_mod_requires_and_bump_preserve_inline_comments() {
    let go_mod = "module example.com/app\n\ngo 1.22\n\nrequire (\n\tgithub.com/foo/pinned v1.2.3 // pinned: see ISSUE-123\n\tgithub.com/foo/upgradable v1.0.0\n)\n\nrequire (\n\tgithub.com/foo/codegen v0.4.0 // indirect; needed for codegen\n\tgithub.com/foo/plain-ind v0.1.0 // indirect\n)\n";
    let bumped = bump_go_mod_in_place(go_mod, "github.com/foo/upgradable", "v1.1.0")
        .expect("bump upgradable module");
    assert!(
        bumped.contains("\tgithub.com/foo/pinned v1.2.3 // pinned: see ISSUE-123\n"),
        "bumping one module must leave sibling inline comments intact:\n{bumped}"
    );
    let merged = merge_go_mod_requires(&bumped);
    assert!(
        merged.contains("\tgithub.com/foo/pinned v1.2.3 // pinned: see ISSUE-123\n"),
        "merge_go_mod_requires must preserve inline comments on direct deps:\n{merged}"
    );
    assert!(
        merged.contains("\tgithub.com/foo/upgradable v1.1.0\n"),
        "merge_go_mod_requires must include updated version:\n{merged}"
    );
    assert!(
        merged.contains("\tgithub.com/foo/codegen v0.4.0 // indirect; needed for codegen\n"),
        "merge_go_mod_requires must preserve extended comments on indirect deps:\n{merged}"
    );
    assert!(
        merged.contains("\tgithub.com/foo/plain-ind v0.1.0 // indirect\n"),
        "merge_go_mod_requires must keep // indirect on standard indirect deps:\n{merged}"
    );
}

#[test]
fn pom_dependencies_scoped_confines_parent_relative_path_for_non_root() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let outside = base.join("outside");
    let workspace = base.join("workspace").join("child");
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(&workspace).unwrap();

    fs::write(
        outside.join("pom.xml"),
        r#"<project>
  <properties>
    <secret.parent.version>9.9.9-LEAKED</secret.parent.version>
  </properties>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.slf4j</groupId>
        <artifactId>slf4j-api</artifactId>
        <version>2.0.99-LEAKED</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
</project>"#,
    )
    .unwrap();

    let child_pom = r#"<project>
  <parent>
    <groupId>com.example</groupId>
    <artifactId>parent</artifactId>
    <version>1.0.0</version>
    <relativePath>../../outside/pom.xml</relativePath>
  </parent>
  <dependencies>
    <dependency>
      <groupId>org.slf4j</groupId>
      <artifactId>slf4j-api</artifactId>
    </dependency>
    <dependency>
      <groupId>com.google.guava</groupId>
      <artifactId>guava</artifactId>
      <version>${secret.parent.version}</version>
    </dependency>
  </dependencies>
</project>"#;
    let child_path = workspace.join("pom.xml");
    fs::write(&child_path, child_pom).unwrap();

    let allowed = vec![base.join("workspace")];
    let non_root_items = parse_pom_dependencies_scoped(&child_path, child_pom, &allowed, false);
    let slf4j = non_root_items
        .iter()
        .find(|i| i.name == "org.slf4j:slf4j-api")
        .unwrap();
    assert_eq!(
        slf4j.current_version, "",
        "non-root caller must not read parent POM outside allowed_roots"
    );
    assert!(slf4j.managed_in.is_none());
    let guava = non_root_items
        .iter()
        .find(|i| i.name == "com.google.guava:guava")
        .unwrap();
    assert_eq!(
        guava.current_version, "${secret.parent.version}",
        "non-root caller must not interpolate properties from parent POM outside allowed_roots"
    );
}
