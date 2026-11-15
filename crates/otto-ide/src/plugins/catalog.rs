//! Built-in curated IDE plugin catalog and plugin lifecycle unit tests.

use crate::types::{IdePluginCatalogEntry, IdePluginManifest};

pub(super) fn builtin_catalog_entries() -> Vec<IdePluginCatalogEntry> {
    vec![
        IdePluginCatalogEntry {
            slug: "kotlin-ide".into(),
            name: "Kotlin & Gradle IDE Pack".into(),
            version: "1.0.0".into(),
            description: "Kotlin (.kt, .kts) language support, kotlin-language-server LSP, kotlin-debug-adapter DAP, JUnit/KotlinTest gutter runners, and Gradle snippets.".into(),
            tags: vec!["kotlin".into(), "jvm".into(), "gradle".into(), "lsp".into(), "dap".into()],
            installed: false,
        },
        IdePluginCatalogEntry {
            slug: "makefile-docker-pack".into(),
            name: "Makefile, Dockerfile & Compose Pack".into(),
            version: "1.0.0".into(),
            description: "Makefile target gutter runners, Dockerfile & Containerfile support, docker-langserver / autotools-language-server LSP bindings, and container snippets.".into(),
            tags: vec!["makefile".into(), "dockerfile".into(), "containers".into(), "lsp".into()],
            installed: false,
        },
        IdePluginCatalogEntry {
            slug: "graphql-ide".into(),
            name: "GraphQL Schema & Operations Pack".into(),
            version: "1.0.0".into(),
            description: "GraphQL (.graphql, .gql) syntax, graphql-lsp stdio integration, query/mutation outline symbols, and Apollo/Relay operation snippets.".into(),
            tags: vec!["graphql".into(), "api".into(), "lsp".into()],
            installed: false,
        },
        IdePluginCatalogEntry {
            slug: "protobuf-grpc-ide".into(),
            name: "Protobuf & gRPC (Buf) Pack".into(),
            version: "1.0.0".into(),
            description: "Protocol Buffers (.proto) language support, buf beta lsp / protols language server, buf format integration, and gRPC service/RPC snippets.".into(),
            tags: vec!["protobuf".into(), "grpc".into(), "buf".into(), "lsp".into()],
            installed: false,
        },
    ]
}

pub(super) fn builtin_catalog_manifest(slug: &str) -> Option<IdePluginManifest> {
    let raw = match slug {
        "kotlin-ide" => {
            include_str!("../../../../examples/ide-plugins/kotlin-ide/otto-ide-plugin.json")
        }
        "makefile-docker-pack" => {
            include_str!(
                "../../../../examples/ide-plugins/makefile-docker-pack/otto-ide-plugin.json"
            )
        }
        "graphql-ide" => GRAPHQL_CATALOG_JSON,
        "protobuf-grpc-ide" => PROTOBUF_CATALOG_JSON,
        _ => return None,
    };
    serde_json::from_str(raw).ok()
}

const GRAPHQL_CATALOG_JSON: &str = r##"{
  "slug": "graphql-ide",
  "name": "GraphQL Schema & Operations Pack",
  "version": "1.0.0",
  "description": "GraphQL (.graphql, .gql) syntax, graphql-lsp stdio integration, query/mutation outline symbols, and operation snippets.",
  "author": "Otto IDE",
  "contributions": {
    "languages": [
      {
        "id": "graphql",
        "display_name": "GraphQL",
        "extensions": [".graphql", ".gql", "schema.graphql"],
        "line_comment": "#"
      }
    ],
    "lsp_servers": [
      {
        "language": "graphql",
        "command": "graphql-lsp",
        "args": ["server", "-m", "stream"],
        "extensions": [".graphql", ".gql"],
        "install_hint": "npm install -g graphql-language-service-cli"
      }
    ],
    "dap_adapters": [],
    "gutter_detectors": [],
    "keymaps": [],
    "snippets": [
      {
        "language": "graphql",
        "prefix": "gqlquery",
        "body": "query GetItem($id: ID!) {\n  item(id: $id) {\n    id\n  }\n}",
        "description": "GraphQL typed query operation"
      }
    ],
    "panels": []
  }
}"##;

const PROTOBUF_CATALOG_JSON: &str = r#"{
  "slug": "protobuf-grpc-ide",
  "name": "Protobuf & gRPC (Buf) Pack",
  "version": "1.0.0",
  "description": "Protocol Buffers (.proto) language support, buf beta lsp / protols language server, buf format integration, and gRPC service snippets.",
  "author": "Otto IDE",
  "contributions": {
    "languages": [
      {
        "id": "protobuf",
        "display_name": "Protocol Buffers",
        "extensions": [".proto", "buf.yaml", "buf.gen.yaml"],
        "line_comment": "//"
      }
    ],
    "lsp_servers": [
      {
        "language": "protobuf",
        "command": "buf",
        "args": ["beta", "lsp"],
        "extensions": [".proto"],
        "install_hint": "brew install bufbuild/buf/buf"
      }
    ],
    "dap_adapters": [],
    "gutter_detectors": [],
    "keymaps": [],
    "snippets": [
      {
        "language": "protobuf",
        "prefix": "grpcservice",
        "body": "service GreeterService {\n  rpc SayHello (HelloRequest) returns (HelloReply);\n}",
        "description": "gRPC service definition"
      }
    ],
    "panels": []
  }
}"#;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::{copy_plugin_dir, IdePluginRegistry};

    #[tokio::test]
    async fn catalog_install_toggle_and_uninstall_lifecycle() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let reg = IdePluginRegistry::with_dirs(tmp.path().to_path_buf(), None);

        let (initial, catalog) = reg.list();
        assert!(initial.is_empty());
        assert_eq!(catalog.len(), 4);
        assert!(catalog.iter().all(|c| !c.installed));

        let kt = reg
            .install("catalog:kotlin-ide")
            .await
            .expect("install kotlin-ide");
        assert_eq!(kt.slug, "kotlin-ide");
        assert!(kt.enabled);
        assert_eq!(kt.contributions.languages.len(), 1);
        assert_eq!(kt.contributions.lsp_servers.len(), 1);
        assert_eq!(kt.contributions.dap_adapters.len(), 1);
        assert_eq!(kt.contributions.gutter_detectors.len(), 2);

        let detected = reg
            .detect_language_for_path("/repo/src/Main.kt")
            .expect("detect .kt");
        assert_eq!(detected.id, "kotlin");
        assert_eq!(reg.active_gutter_detectors().len(), 2);

        let disabled = reg.toggle("kotlin-ide", false).expect("disable");
        assert!(!disabled.enabled);
        assert!(reg.detect_language_for_path("/repo/src/Main.kt").is_none());
        assert!(reg.active_gutter_detectors().is_empty());

        let reg2 = IdePluginRegistry::with_dirs(tmp.path().to_path_buf(), None);
        assert!(reg2.detect_language_for_path("/repo/src/Main.kt").is_none());

        let enabled = reg2.toggle("kotlin-ide", true).expect("re-enable");
        assert!(enabled.enabled);
        assert!(reg2
            .detect_language_for_path("/repo/build.gradle.kts")
            .is_some());

        reg2.uninstall("kotlin-ide").expect("uninstall");
        let (after, _) = reg2.list();
        assert!(after.is_empty());
    }

    #[tokio::test]
    async fn local_path_and_sidecar_otto_plugin_discovery() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let data_dir = tmp.path().join("data");
        let sidecar_dir = tmp.path().join("otto-plugins");
        std::fs::create_dir_all(sidecar_dir.join("my-sidecar")).unwrap();
        std::fs::create_dir_all(sidecar_dir.join("plain-sidecar")).unwrap();

        std::fs::write(
            sidecar_dir.join("my-sidecar/otto-plugin.json"),
            r#"{
              "slug": "my-sidecar",
              "name": "My Sidecar + IDE",
              "version": "0.3.0",
              "exec": ["node", "server.js"],
              "ide": {
                "languages": [{ "id": "cue", "display_name": "CUE", "extensions": [".cue"] }],
                "lsp_servers": [{ "language": "cue", "command": "cue", "args": ["lsp"], "extensions": [".cue"] }]
              }
            }"#,
        )
        .unwrap();

        std::fs::write(
            sidecar_dir.join("plain-sidecar/otto-plugin.json"),
            r#"{"slug":"plain-sidecar","name":"Plain","exec":["./run"]}"#,
        )
        .unwrap();

        let reg = IdePluginRegistry::with_dirs(data_dir, Some(sidecar_dir));
        let (plugins, _) = reg.list();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].slug, "my-sidecar");
        assert!(reg.detect_language_for_path("policy.cue").is_some());

        let local_pack = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/ide-plugins/makefile-docker-pack");
        let installed = reg
            .install(&local_pack.to_string_lossy())
            .await
            .expect("install local pack");
        assert_eq!(installed.slug, "makefile-docker-pack");
        assert!(reg.detect_language_for_path("/app/Makefile").is_some());
        assert!(reg.detect_language_for_path("/app/Dockerfile").is_some());
    }

    #[tokio::test]
    async fn rejects_unsafe_sources_and_non_https_git_urls() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let reg = IdePluginRegistry::with_dirs(tmp.path().to_path_buf(), None);

        assert!(reg.install("--upload-pack=evil").await.is_err());
        assert!(reg.install("-oProxyCommand=evil").await.is_err());
        assert!(reg.install("http://example.com/repo.git").await.is_err());
        assert!(reg
            .install("https://127.0.0.1:8443/repo.git")
            .await
            .is_err());
        assert!(reg.install("https://10.0.0.5/internal.git").await.is_err());
        assert!(reg.install("ssh://git@example.com/repo.git").await.is_err());
        assert!(reg.install("file:///tmp/repo").await.is_err());
        assert!(reg.toggle("../traversal", false).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn copy_plugin_dir_strips_symlinks_git_metadata_and_build_artifacts() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let staged = tmp.path().join("staged-git-clone");
        std::fs::create_dir_all(staged.join(".git")).unwrap();
        std::fs::create_dir_all(staged.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(staged.join("target/debug")).unwrap();
        std::fs::write(staged.join(".git/config"), "[core]\n").unwrap();
        std::fs::write(staged.join("node_modules/pkg/index.js"), "1\n").unwrap();
        std::fs::write(staged.join("target/debug/bin"), "bin\n").unwrap();
        std::fs::write(
            staged.join("otto-ide-plugin.json"),
            r#"{"slug":"clean-pack","name":"Clean Pack","version":"1.0.0","contributions":{}}"#,
        )
        .unwrap();
        let outside = tmp.path().join("outside.txt");
        std::fs::write(&outside, "secret").unwrap();
        std::os::unix::fs::symlink(&outside, staged.join("escaped-link.txt")).unwrap();

        let dst = tmp.path().join("dest/clean-pack");
        copy_plugin_dir(&staged, &dst).expect("copy staged plugin");
        assert!(dst.join("otto-ide-plugin.json").is_file());
        assert!(
            std::fs::symlink_metadata(dst.join("escaped-link.txt")).is_err(),
            "symlinks inside cloned/local plugins must be stripped"
        );
        assert!(!dst.join(".git").exists());
        assert!(!dst.join("node_modules").exists());
        assert!(!dst.join("target").exists());
    }
}
