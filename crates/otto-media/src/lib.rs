//! `otto-media` — Native 3D Asset Generation Providers (Tencent Hunyuan3D-Omni,
//! Microsoft TRELLIS.2, Custom HTTP/CLI — zero ComfyUI), Built-in & Extensible
//! TTS Engine (zero-setup macOS `say` + OpenAI-compatible / ElevenLabs / Kokoro
//! / Custom HTTP), and Workspace-Confined Asset Export.

pub mod audio;
pub mod export;
pub mod gen3d;
pub mod http;
pub mod net;
pub mod productions;
pub mod service;
pub mod store;
pub mod tts;
pub mod types;

pub use http::{api_router, MediaCtx};
pub use service::MediaService;
pub use store::MediaStore;

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use axum::{Extension, Router};
    use chrono::Utc;
    use http_body_util::BodyExt;
    use otto_core::auth::{
        with_request_credential, AuthUser, BoxFuture, RequestCredential, RoleChecker,
    };
    use otto_core::domain::{User, WorkspaceRole};
    use otto_core::secrets::SecretStore;
    use otto_core::{Error, Result};
    use otto_design::DesignService;
    use otto_state::WorkspacesRepo;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::gen3d::{
        hunyuan3d_omni::build_hunyuan_http_payload, trellis2::build_trellis2_http_payload,
        Gen3dDriver, Gen3dOutput, Gen3dRequest,
    };
    use crate::http::{api_router, MediaCtx};
    use crate::service::MediaService;
    use crate::tts::{TtsDriver, TtsSynthOutput, TtsSynthRequest};
    use crate::types::{
        Media3dProviderRecord, ProviderTestResult, TtsProviderRecord, TtsVisemeCue,
        TtsVisemeTimeline, TtsVoice, BUILTIN_HUNYUAN_ID, BUILTIN_MACOS_SAY_ID,
        BUILTIN_OTTO_FORGE_ID, BUILTIN_TRELLIS2_ID, DEFAULT_3D_MAX_OUTPUT_BYTES,
        MAX_3D_TIMEOUT_SECS,
    };

    #[derive(Default)]
    struct MemSecrets(Mutex<HashMap<String, String>>);

    impl SecretStore for MemSecrets {
        fn put(&self, key: &str, value: &str) -> Result<()> {
            self.0
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }
        fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }
        fn delete(&self, key: &str) -> Result<()> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }

    struct AllowAllRoles;
    impl RoleChecker for AllowAllRoles {
        fn check<'a>(
            &'a self,
            user: &'a User,
            _workspace_id: &'a otto_core::Id,
            _min: WorkspaceRole,
        ) -> BoxFuture<'a, Result<()>> {
            let ok = user.id != "denied";
            Box::pin(async move {
                if ok {
                    Ok(())
                } else {
                    Err(Error::Forbidden("denied".into()))
                }
            })
        }
    }

    struct Fake3dDriver {
        valid_glb: Vec<u8>,
    }

    #[async_trait]
    impl Gen3dDriver for Fake3dDriver {
        async fn generate(
            &self,
            req: &Gen3dRequest,
            _provider: &Media3dProviderRecord,
            _api_key: Option<&str>,
            cancel: &AtomicBool,
        ) -> Result<Gen3dOutput> {
            if req.prompt.as_deref() == Some("finish after cancel") {
                // A provider that finishes as its job is cancelled.
                for _ in 0..200 {
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
            }
            if req.pipeline_stage != "generate" && req.source_mesh_bytes.is_none() {
                return Err(Error::Invalid("a transform stage got no mesh".into()));
            }
            let bytes = if req.output_format == "obj" {
                b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()
            } else {
                // Each stage writes different bytes, as a real one does (an
                // identical save would commit no version).
                let mut b = self.valid_glb.clone();
                if req.pipeline_stage != "generate" {
                    b.extend_from_slice(req.pipeline_stage.as_bytes());
                }
                b
            };
            Ok(Gen3dOutput {
                bytes,
                format: req.output_format.clone(),
                manifest: Some(match req.prompt.as_deref() {
                    // Forge repeats the prompt in its manifest; a long one outgrows provenance.
                    Some("oversize manifest") => {
                        json!({ "triangles": 4200, "pad": "x".repeat(20_000) })
                    }
                    _ => json!({ "stage": req.pipeline_stage, "triangles": 4200 }),
                }),
            })
        }

        async fn test_connection(
            &self,
            provider: &Media3dProviderRecord,
            _api_key: Option<&str>,
        ) -> Result<ProviderTestResult> {
            Ok(ProviderTestResult {
                ok: true,
                latency_ms: Some(5),
                message: format!("fake ok ({})", provider.kind),
                detected_version: Some("1.0".into()),
                details: None,
            })
        }
    }

    /// Minimal valid 44-byte RIFF/WAVE header + 100 bytes PCM.
    fn tiny_wav() -> Vec<u8> {
        let mut wav = Vec::with_capacity(144);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(136u32).to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&(16u32).to_le_bytes());
        wav.extend_from_slice(&(1u16).to_le_bytes()); // PCM
        wav.extend_from_slice(&(1u16).to_le_bytes()); // mono
        wav.extend_from_slice(&(22050u32).to_le_bytes());
        wav.extend_from_slice(&(44100u32).to_le_bytes()); // byte_rate
        wav.extend_from_slice(&(2u16).to_le_bytes());
        wav.extend_from_slice(&(16u16).to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(100u32).to_le_bytes());
        wav.extend_from_slice(&[0u8; 100]);
        wav
    }

    struct FakeTtsDriver;

    #[async_trait]
    impl TtsDriver for FakeTtsDriver {
        async fn list_voices(
            &self,
            provider: &TtsProviderRecord,
            _api_key: Option<&str>,
        ) -> Result<Vec<TtsVoice>> {
            // Samantha, then voices the way macOS names them (no gender).
            let mut voices = vec![TtsVoice {
                id: "Samantha".into(),
                name: "Samantha".into(),
                locale: Some("en_US".into()),
                gender: Some("female".into()),
                preview_text: Some("Hello!".into()),
                provider_id: provider.id.clone(),
            }];
            for id in ["Albert", "Eddy (English (US))", "Fred"] {
                voices.push(TtsVoice {
                    id: id.into(),
                    name: id.into(),
                    locale: Some("en_US".into()),
                    gender: None,
                    preview_text: None,
                    provider_id: provider.id.clone(),
                });
            }
            Ok(voices)
        }

        async fn synthesize(
            &self,
            req: &TtsSynthRequest,
            _provider: &TtsProviderRecord,
            _api_key: Option<&str>,
        ) -> Result<TtsSynthOutput> {
            Ok(TtsSynthOutput {
                bytes: tiny_wav(),
                format: req.format.clone(),
                mime: "audio/wav".into(),
                duration_ms: Some(2),
                visemes: Some(TtsVisemeTimeline {
                    duration_sec: 0.42,
                    cues: vec![
                        TtsVisemeCue {
                            start: 0.0,
                            end: 0.2,
                            value: "A".into(),
                        },
                        TtsVisemeCue {
                            start: 0.2,
                            end: 0.42,
                            value: "O".into(),
                        },
                    ],
                }),
            })
        }

        async fn test_connection(
            &self,
            _provider: &TtsProviderRecord,
            _api_key: Option<&str>,
        ) -> Result<ProviderTestResult> {
            Ok(ProviderTestResult {
                ok: true,
                latency_ms: Some(2),
                message: "fake tts ok".into(),
                detected_version: None,
                details: None,
            })
        }
    }

    #[derive(Clone)]
    struct TestCtx {
        media: Arc<MediaService>,
        pool: otto_state::DbPool,
        data_dir: std::path::PathBuf,
        roles: Arc<dyn RoleChecker>,
        workspaces: WorkspacesRepo,
        planner: Arc<production_flows::FakePlanner>,
        /// What the host checks a pick against (the built-in providers).
        providers: Arc<otto_sessions::ProviderRegistry>,
        /// Each planner the router asked for, as given.
        picks: Arc<Mutex<Vec<Pick>>>,
    }

    /// The pick as the router passed it to the host.
    type Pick = crate::productions::AgentPick;

    impl MediaCtx for TestCtx {
        fn media(&self) -> &Arc<MediaService> {
            &self.media
        }
        fn design(&self) -> DesignService {
            DesignService::new(self.pool.clone(), self.data_dir.clone(), None)
        }
        fn roles(&self) -> &Arc<dyn RoleChecker> {
            &self.roles
        }
        fn workspaces(&self) -> &WorkspacesRepo {
            &self.workspaces
        }
        /// The host's check (the workspace default is `claude` here), then
        /// the fake planner; the pick is recorded as the host received it.
        async fn production_planner(
            &self,
            _ws: &otto_core::domain::Workspace,
            _user: &User,
            pick: &Pick,
        ) -> Result<Arc<dyn crate::productions::ProductionPlanner>> {
            let agent = pick.provider.as_deref().unwrap_or("claude");
            let (model, effort) = (pick.model.as_deref(), pick.effort.as_deref());
            crate::productions::agent::check_agent(&self.providers, agent, model, effort).await?;
            self.picks.lock().unwrap().push(pick.clone());
            Ok(self.planner.clone())
        }
    }

    fn test_user(id: &str) -> User {
        User {
            id: id.into(),
            username: id.into(),
            display_name: id.into(),
            is_root: true,
            disabled: false,
            created_at: Utc::now(),
        }
    }

    fn minimal_glb() -> Vec<u8> {
        let mut b = b"glTF".to_vec();
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&12u32.to_le_bytes());
        b
    }

    async fn setup_test(
        use_fakes: bool,
    ) -> (
        TestCtx,
        String,
        tempfile::TempDir,
        tempfile::TempDir,
        Arc<MemSecrets>,
    ) {
        setup_with(use_fakes, use_fakes).await
    }

    async fn setup_with(
        fake_3d: bool,
        fake_tts: bool,
    ) -> (
        TestCtx,
        String,
        tempfile::TempDir,
        tempfile::TempDir,
        Arc<MemSecrets>,
    ) {
        let pool = otto_state::db::test_pool().await;
        sqlx::query(
            "INSERT INTO users (id, username, display_name, password_hash, is_root, disabled, created_at)
             VALUES ('u1', 'u1', 'Test User', 'x', 1, 0, '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let data_dir = tempfile::tempdir().unwrap();
        let ws_dir = tempfile::tempdir().unwrap();
        let workspaces = WorkspacesRepo::new(pool.clone());
        let ws = workspaces
            .create(
                "Game Workspace",
                ws_dir.path().to_str().unwrap(),
                &"u1".to_string(),
            )
            .await
            .unwrap();
        let ws_id = ws.id;
        let secrets = Arc::new(MemSecrets::default());
        let mut svc = MediaService::new(
            pool.clone(),
            data_dir.path().to_path_buf(),
            secrets.clone(),
            None,
        );
        if fake_3d {
            svc = svc.with_gen3d_driver(Arc::new(Fake3dDriver {
                valid_glb: minimal_glb(),
            }));
        }
        if fake_tts {
            svc = svc.with_tts_driver(Arc::new(FakeTtsDriver));
        }
        let ctx = TestCtx {
            media: Arc::new(svc),
            pool,
            data_dir: data_dir.path().to_path_buf(),
            roles: Arc::new(AllowAllRoles),
            workspaces,
            planner: Default::default(),
            providers: Arc::new(otto_sessions::ProviderRegistry::new(None)),
            picks: Default::default(),
        };
        (ctx, ws_id, data_dir, ws_dir, secrets)
    }

    async fn call_json(
        app: &Router,
        method: Method,
        uri: &str,
        body: Option<Value>,
        is_agent: bool,
    ) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(match body {
                Some(v) => Body::from(serde_json::to_vec(&v).unwrap()),
                None => Body::empty(),
            })
            .unwrap();
        let cred = RequestCredential {
            agent: is_agent,
            root_withheld: is_agent,
        };
        let resp = with_request_credential(cred, app.clone().oneshot(req))
            .await
            .unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let val = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, val)
    }

    #[tokio::test]
    async fn hunyuan_and_trellis2_payload_builders_honor_real_parameters() {
        let now = Utc::now();
        let hunyuan = Media3dProviderRecord {
            id: BUILTIN_HUNYUAN_ID.into(),
            name: "Tencent Hunyuan3D-Omni".into(),
            kind: "hunyuan3d_omni".into(),
            transport: "http".into(),
            builtin: true,
            enabled: true,
            license_warning: Some("territory warning".into()),
            endpoint_url: Some("http://127.0.0.1:8080/generate".into()),
            cli_binary: None,
            cli_args_template: vec![],
            cli_workdir: None,
            auth_header: None,
            secret_key_ref: None,
            allow_local_network: true,
            timeout_secs: 600,
            max_output_bytes: DEFAULT_3D_MAX_OUTPUT_BYTES,
            default_params: json!({ "use_ema": true, "flashvdm": true }),
            created_at: now,
            updated_at: now,
        };
        let req = Gen3dRequest {
            job_id: "j1".into(),
            pipeline_stage: "generate".into(),
            prompt: Some("Stylized knight helmet".into()),
            image_bytes: Some(vec![1, 2, 3]),
            image_mime: Some("image/png".into()),
            source_mesh_bytes: None,
            source_mesh_format: None,
            output_format: "glb".into(),
            poly_budget: Some(15_000),
            seed: Some(7),
            params: json!({ "control_type": "bbox", "bbox": [1.0, 2.0, 1.0] }),
            max_output_bytes: DEFAULT_3D_MAX_OUTPUT_BYTES as usize,
            timeout: std::time::Duration::from_secs(60),
            progress: None,
        };
        let hp = build_hunyuan_http_payload(&req, &hunyuan).unwrap();
        assert_eq!(hp["control_type"], "bbox");
        assert_eq!(hp["use_ema"], true);
        assert_eq!(hp["flashvdm"], true);
        assert_eq!(hp["face_count"], 15_000);
        assert_eq!(hp["seed"], 7);

        let trellis = Media3dProviderRecord {
            id: BUILTIN_TRELLIS2_ID.into(),
            kind: "trellis_2".into(),
            ..hunyuan
        };
        let treq = Gen3dRequest {
            params: json!({ "resolution": 1536, "texture_size": 4096, "remesh": true }),
            ..req
        };
        let tp = build_trellis2_http_payload(&treq, &trellis).unwrap();
        assert_eq!(tp["resolution"], 1536);
        assert_eq!(tp["decimation_target"], 15_000);
        assert_eq!(tp["texture_size"], 4096);
        assert_eq!(tp["pbr"], true);
    }

    async fn wait_job(app: &Router, jid: &str) -> Value {
        for _ in 0..40 {
            let (_, j) = call_json(
                app,
                Method::GET,
                &format!("/design/3d/jobs/{jid}"),
                None,
                false,
            )
            .await;
            if j["status"] != "queued" && j["status"] != "running" {
                return j;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("job {jid} did not finish");
    }

    async fn first_version_provenance(pool: &otto_state::DbPool, artifact_id: &str) -> Value {
        let raw: String = sqlx::query_scalar(
            "SELECT provenance_json FROM design_versions WHERE artifact_id = ? ORDER BY seq LIMIT 1",
        )
        .bind(artifact_id)
        .fetch_one(pool)
        .await
        .unwrap();
        serde_json::from_str(&raw).unwrap()
    }

    /// A generate job that creates a new artifact records the generator and
    /// the CLI manifest on its first version, as a stage job does; a manifest
    /// over otto-design's provenance cap is left on the job, not the version.
    #[tokio::test]
    async fn generated_artifact_version_keeps_manifest_provenance() {
        let (ctx, ws_id, _dd, _ws_dir, _secrets) = setup_test(true).await;
        let app = api_router::<TestCtx>()
            .layer(Extension(AuthUser(test_user("u1"))))
            .with_state(ctx.clone());
        for prompt in ["Knight from a concept image", "oversize manifest"] {
            let (st, job) = call_json(
                &app,
                Method::POST,
                "/design/3d/jobs",
                Some(json!({
                    "workspace_id": ws_id,
                    "provider_id": BUILTIN_OTTO_FORGE_ID,
                    "prompt": prompt,
                    "output_format": "glb"
                })),
                false,
            )
            .await;
            assert_eq!(st, StatusCode::ACCEPTED, "{job}");
            let done = wait_job(&app, job["id"].as_str().unwrap()).await;
            assert_eq!(done["status"], "succeeded", "{done}");
            assert_eq!(done["manifest"]["triangles"], 4200);
            let prov =
                first_version_provenance(&ctx.pool, done["artifact_id"].as_str().unwrap()).await;
            assert_eq!(
                prov["generator"]["provider_id"], BUILTIN_OTTO_FORGE_ID,
                "{prov}"
            );
            if prompt == "oversize manifest" {
                assert!(prov["manifest"]["omitted"].is_string(), "{prov}");
            } else {
                assert_eq!(prov["manifest"]["triangles"], 4200, "{prov}");
            }
        }
    }

    #[tokio::test]
    async fn full_3d_provider_job_versioning_export_and_agent_guard_flow() {
        let (ctx, ws_id, _dd, ws_dir, secrets) = setup_test(true).await;
        let app = api_router::<TestCtx>()
            .layer(Extension(AuthUser(test_user("u1"))))
            .with_state(ctx.clone());

        // Built-in Otto Forge, TRELLIS.2, and Hunyuan3D-Omni exist out of the box;
        // Hunyuan3D-Omni is NOT default (ordered last) and carries territory license_warning.
        let (st, list) = call_json(&app, Method::GET, "/design/3d/providers", None, false).await;
        assert_eq!(st, StatusCode::OK);
        let arr = list.as_array().unwrap();
        assert_eq!(arr[0]["id"], BUILTIN_OTTO_FORGE_ID);
        assert_eq!(arr[1]["id"], BUILTIN_TRELLIS2_ID);
        assert_eq!(arr[2]["id"], BUILTIN_HUNYUAN_ID);
        assert!(arr[2]["license_warning"]
            .as_str()
            .unwrap()
            .contains("EU, UK, and South Korea"));
        assert_eq!(arr[0]["max_output_bytes"], DEFAULT_3D_MAX_OUTPUT_BYTES);
        // A local `forge generate` can take ~25 min: Forge gets the cap.
        assert_eq!(arr[0]["timeout_secs"], MAX_3D_TIMEOUT_SECS);
        assert_eq!(
            arr[0]["capabilities"]["supported_stages"],
            json!(["generate", "gameready", "rig", "animate"])
        );

        // Agent credential is blocked from registering or modifying providers.
        let (st, _) = call_json(
            &app,
            Method::POST,
            "/design/3d/providers",
            Some(json!({
                "name": "Sneaky CLI",
                "kind": "custom_cli",
                "transport": "local_cli",
                "cli_binary": "/bin/echo"
            })),
            true,
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN);

        // Human registers custom HTTP provider with API key stored in Keychain only.
        let (st, created) = call_json(
            &app,
            Method::POST,
            "/design/3d/providers",
            Some(json!({
                "name": "Studio GPU Box",
                "kind": "custom_http",
                "transport": "http",
                "endpoint_url": "https://gpu.example.com/v1/3d",
                "api_key": "super-secret-3d-key"
            })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{created}");
        assert_eq!(created["has_api_key"], true);
        assert!(!created.to_string().contains("super-secret-3d-key"));
        let pid = created["id"].as_str().unwrap().to_string();
        assert!(secrets
            .0
            .lock()
            .unwrap()
            .contains_key(&crate::types::secret_key_3d(&pid)));

        // Start a 3D generation job -> auto-saves versioned Design Hall GLB artifact!
        let (st, job) = call_json(
            &app,
            Method::POST,
            "/design/3d/jobs",
            Some(json!({
                "workspace_id": ws_id,
                "provider_id": BUILTIN_HUNYUAN_ID,
                "title": "Knight Shield",
                "prompt": "Low-poly heater shield with iron trim",
                "preset": "low_poly_prop",
                "output_format": "glb",
                "seed": 42
            })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::ACCEPTED, "{job}");
        let jid = job["id"].as_str().unwrap().to_string();

        // Poll until succeeded (fake driver finishes immediately).
        let mut final_job = Value::Null;
        for _ in 0..40 {
            let (st, j) = call_json(
                &app,
                Method::GET,
                &format!("/design/3d/jobs/{jid}"),
                None,
                false,
            )
            .await;
            assert_eq!(st, StatusCode::OK);
            if j["status"] == "succeeded" {
                final_job = j;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(final_job["status"], "succeeded", "{final_job}");
        assert_eq!(final_job["manifest"]["triangles"], 4200);
        let aid = final_job["artifact_id"].as_str().unwrap();

        // Run a stage job (`rig`) on the generated artifact using `otto-forge`:
        // verifies stage validation, source mesh materialization, and versioning onto same artifact.
        let (st, rig_job) = call_json(
            &app,
            Method::POST,
            "/design/3d/jobs",
            Some(json!({
                "workspace_id": ws_id,
                "provider_id": BUILTIN_OTTO_FORGE_ID,
                "stage": "rig",
                "source_artifact_id": aid,
                "output_format": "glb"
            })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::ACCEPTED, "{rig_job}");
        assert_eq!(rig_job["pipeline_stage"], "rig");
        assert_eq!(rig_job["source_artifact_id"], aid);
        assert_eq!(rig_job["target_artifact_id"], aid);
        let rjid = rig_job["id"].as_str().unwrap().to_string();
        let mut final_rig = Value::Null;
        for _ in 0..40 {
            let (_, j) = call_json(
                &app,
                Method::GET,
                &format!("/design/3d/jobs/{rjid}"),
                None,
                false,
            )
            .await;
            if j["status"] == "succeeded" {
                final_rig = j;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(final_rig["status"], "succeeded", "{final_rig}");
        assert_eq!(final_rig["artifact_id"], aid);
        assert_eq!(final_rig["manifest"]["stage"], "rig");

        // Export generated GLB into confined workspace path.
        let (st, exp) = call_json(
            &app,
            Method::POST,
            &format!("/design/3d/artifacts/{aid}/export-workspace"),
            Some(json!({ "relative_path": "assets/models/shield.glb" })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{exp}");
        assert!(ws_dir.path().join("assets/models/shield.glb").is_file());

        // Traversal attempt is rejected.
        let (st, _) = call_json(
            &app,
            Method::POST,
            &format!("/design/3d/artifacts/{aid}/export-workspace"),
            Some(json!({ "relative_path": "../../evil.glb" })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn full_tts_synthesize_audio_stream_export_and_delete_flow() {
        let (ctx, ws_id, _dd, ws_dir, _secrets) = setup_test(true).await;
        let app = api_router::<TestCtx>()
            .layer(Extension(AuthUser(test_user("u1"))))
            .with_state(ctx.clone());

        // Built-in macOS TTS provider exists and cannot be deleted.
        let (st, providers) =
            call_json(&app, Method::GET, "/product/tts/providers", None, false).await;
        assert_eq!(st, StatusCode::OK);
        assert!(providers
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == BUILTIN_MACOS_SAY_ID));

        let (st, _) = call_json(
            &app,
            Method::DELETE,
            &format!("/product/tts/providers/{BUILTIN_MACOS_SAY_ID}"),
            None,
            false,
        )
        .await;
        assert_eq!(st, StatusCode::CONFLICT);

        // List voices.
        let (st, voices) = call_json(
            &app,
            Method::GET,
            &format!("/product/tts/providers/{BUILTIN_MACOS_SAY_ID}/voices"),
            None,
            false,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(voices.as_array().unwrap()[0]["id"], "Samantha");

        // Synthesize game dialogue clip.
        let (st, clip) = call_json(
            &app,
            Method::POST,
            "/product/tts/synthesize",
            Some(json!({
                "workspace_id": ws_id,
                "voice_id": "Samantha",
                "character_name": "Commander",
                "text": "Hold the western gate until dawn!",
                "speed": 1.1,
                "format": "wav"
            })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{clip}");
        assert_eq!(clip["visemes"]["duration_sec"], 0.42);
        assert_eq!(clip["visemes"]["cues"].as_array().unwrap().len(), 2);
        let cid = clip["id"].as_str().unwrap().to_string();

        // Stream raw audio bytes with nosniff + audio/wav.
        let req = Request::builder()
            .method(Method::GET)
            .uri(format!("/product/tts/clips/{cid}/audio"))
            .body(Body::empty())
            .unwrap();
        let resp = with_request_credential(
            RequestCredential {
                agent: false,
                root_withheld: false,
            },
            app.clone().oneshot(req),
        )
        .await
        .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("content-type").unwrap(), "audio/wav");
        assert_eq!(
            resp.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );

        // Export audio clip to workspace.
        let (st, exp) = call_json(
            &app,
            Method::POST,
            &format!("/product/tts/clips/{cid}/export-workspace"),
            Some(json!({ "relative_path": "assets/audio/gate_line.wav" })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{exp}");
        assert!(ws_dir.path().join("assets/audio/gate_line.wav").is_file());

        // Delete clip cleans up DB row + disk file.
        let (st, _) = call_json(
            &app,
            Method::DELETE,
            &format!("/product/tts/clips/{cid}"),
            None,
            false,
        )
        .await;
        assert_eq!(st, StatusCode::NO_CONTENT);
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn real_macos_say_lists_voices_and_synthesizes_wav() {
        if !std::path::Path::new("/usr/bin/say").is_file() {
            return;
        }
        let (ctx, ws_id, _dd, _ws, _sec) = setup_test(false).await;
        let voices = ctx
            .media
            .list_tts_voices(BUILTIN_MACOS_SAY_ID)
            .await
            .unwrap();
        assert!(!voices.is_empty(), "expected macOS voices from `say -v ?`");
        let voice = voices
            .iter()
            .find(|v| v.locale.as_deref() == Some("en_US"))
            .unwrap_or(&voices[0]);
        let clip = ctx
            .media
            .synthesize_tts(
                crate::types::SynthesizeTtsReq {
                    workspace_id: ws_id,
                    story_id: None,
                    artifact_id: None,
                    provider_id: Some(BUILTIN_MACOS_SAY_ID.into()),
                    voice_id: voice.id.clone(),
                    text: "Otto native speech synthesis test.".into(),
                    speed: Some(1.0),
                    format: Some("wav".into()),
                    title: Some("Real macOS WAV".into()),
                    character_name: Some("Narrator".into()),
                    model: None,
                    voice_settings: None,
                },
                "u1",
            )
            .await
            .unwrap();
        assert_eq!(clip.format, "wav");
        assert!(clip.size_bytes > 1000, "expected real WAV bytes");
        assert!(clip.duration_ms.unwrap_or(0) > 100);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn real_custom_cli_stage_job_materializes_mesh_and_ingests_manifest() {
        use std::os::unix::fs::PermissionsExt;

        let (ctx, ws_id, _dd, _ws, _sec) = setup_test(false).await;
        let script_dir = tempfile::tempdir().unwrap();
        let script_path = script_dir.path().join("fake_forge.sh");
        // Strictly argv-invoked executable that copies `{mesh}` -> `{output}` and writes `<output>.manifest.json`.
        std::fs::write(
            &script_path,
            "#!/bin/sh\nset -e\nSTAGE=\"$1\"\nMESH=\"$2\"\nOUT=\"$3\"\ncp \"$MESH\" \"$OUT\"\nprintf '{\"stage\":\"%s\",\"lod_triangles\":4800,\"rigged\":true}' \"$STAGE\" > \"${OUT}.manifest.json\"\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();

        let app = api_router::<TestCtx>()
            .layer(Extension(AuthUser(test_user("u1"))))
            .with_state(ctx.clone());

        // Seed an initial v1 GLB artifact in Design Hall.
        let v1 = ctx
            .design()
            .create_artifact(otto_design::service::CreateInput {
                workspace_id: ws_id.clone(),
                project_id: None,
                studio: Some("3d".into()),
                format: "glb".into(),
                title: "Hero Character".into(),
                tags: vec![],
                meta: json!({}),
                content: Some(minimal_glb()),
                story_id: None,
                derived_from: None,
                created_by: "u1".into(),
                author: otto_design::service::Author::user("u1"),
                message: Some("initial".into()),
                source: None,
                created_at: None,
                version_kind: Some("named".into()),
                validate: true,
                provenance: None,
            })
            .await
            .unwrap();
        let aid = v1.artifact.id;

        // Configure built-in `otto-forge` with our temp executable + `{stage}` / `{mesh}` / `{output}`.
        let (st, patched) = call_json(
            &app,
            Method::PATCH,
            &format!("/design/3d/providers/{BUILTIN_OTTO_FORGE_ID}"),
            Some(json!({
                "cli_binary": script_path.to_str().unwrap(),
                "cli_args_template": ["{stage}", "{mesh}", "{output}"]
            })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{patched}");
        assert_eq!(patched["ready"], true);

        // Trigger a `gameready` stage job on `aid`.
        let (st, job) = call_json(
            &app,
            Method::POST,
            "/design/3d/jobs",
            Some(json!({
                "workspace_id": ws_id,
                "provider_id": BUILTIN_OTTO_FORGE_ID,
                "stage": "gameready",
                "source_artifact_id": aid,
                "output_format": "glb"
            })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::ACCEPTED, "{job}");
        let jid = job["id"].as_str().unwrap().to_string();

        let mut final_job = Value::Null;
        for _ in 0..80 {
            let (_, j) = call_json(
                &app,
                Method::GET,
                &format!("/design/3d/jobs/{jid}"),
                None,
                false,
            )
            .await;
            if j["status"] == "succeeded" || j["status"] == "failed" {
                final_job = j;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(final_job["status"], "succeeded", "{final_job}");
        assert_eq!(final_job["artifact_id"], aid);
        assert_eq!(final_job["manifest"]["stage"], "gameready");
        assert_eq!(final_job["manifest"]["lod_triangles"], 4800);
        assert_eq!(final_job["manifest"]["rigged"], true);
    }

    /// Write an executable test script and return its path.
    #[cfg(unix)]
    fn write_script(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// A local CLI's JSON Lines progress on stderr moves the job row
    /// (monotonic, below 100 until committed), and its `{"error"}` line is the
    /// failure message.
    #[cfg(unix)]
    #[tokio::test]
    async fn cli_progress_lines_drive_the_job_row_and_error_line_is_the_failure() {
        let (ctx, ws_id, _dd, _ws, _sec) = setup_test(false).await;
        let dir = tempfile::tempdir().unwrap();
        let script = write_script(
            dir.path(),
            "fake_forge.sh",
            "#!/bin/sh\nset -e\nSTAGE=\"$1\"\nMESH=\"$2\"\nOUT=\"$3\"\n\
             progress() { echo \"$1\" >&2; sleep 1; }\n\
             if [ \"$STAGE\" = rig ]; then\n\
               echo '{\"stage\":\"rig\",\"pct\":5,\"message\":\"fitting a skeleton\"}' >&2\n\
               echo 'Traceback (most recent call last): boom' >&2\n\
               echo '{\"error\": \"SkinTokens weights are missing\"}' >&2\n\
               exit 1\n\
             fi\n\
             sleep 1\n\
             progress '{\"stage\":\"gameready\",\"pct\":30,\"message\":\"Baking ORM maps\"}'\n\
             echo 'Blender 5.2.2 starting' >&2\n\
             progress '{\"stage\":\"gameready\",\"pct\":60,\"message\":\"Building LODs\"}'\n\
             progress '{\"stage\":\"gameready\",\"pct\":10,\"message\":\"Packing collision\"}'\n\
             progress '{\"stage\":\"gameready\",\"pct\":100,\"message\":\"wrote output\"}'\n\
             cp \"$MESH\" \"$OUT\"\n",
        );
        let app = api_router::<TestCtx>()
            .layer(Extension(AuthUser(test_user("u1"))))
            .with_state(ctx.clone());
        let (st, patched) = call_json(
            &app,
            Method::PATCH,
            &format!("/design/3d/providers/{BUILTIN_OTTO_FORGE_ID}"),
            Some(json!({
                "cli_binary": script.to_str().unwrap(),
                "cli_args_template": ["{stage}", "{mesh}", "{output}"]
            })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{patched}");
        let aid = ctx
            .design()
            .create_artifact(otto_design::service::CreateInput {
                workspace_id: ws_id.clone(),
                project_id: None,
                studio: Some("3d".into()),
                format: "glb".into(),
                title: "Crate".into(),
                tags: vec![],
                meta: json!({}),
                content: Some(minimal_glb()),
                story_id: None,
                derived_from: None,
                created_by: "u1".into(),
                author: otto_design::service::Author::user("u1"),
                message: Some("initial".into()),
                source: None,
                created_at: None,
                version_kind: Some("named".into()),
                validate: true,
                provenance: None,
            })
            .await
            .unwrap()
            .artifact
            .id;

        let start = |stage: &'static str| {
            let app = app.clone();
            let (ws_id, aid) = (ws_id.clone(), aid.clone());
            async move {
                let (st, job) = call_json(
                    &app,
                    Method::POST,
                    "/design/3d/jobs",
                    Some(json!({
                        "workspace_id": ws_id,
                        "provider_id": BUILTIN_OTTO_FORGE_ID,
                        "stage": stage,
                        "source_artifact_id": aid,
                        "output_format": "glb"
                    })),
                    false,
                )
                .await;
                assert_eq!(st, StatusCode::ACCEPTED, "{job}");
                job["id"].as_str().unwrap().to_string()
            }
        };

        // Record every distinct (pct, stage) the job row passes through.
        let jid = start("gameready").await;
        let mut seen: Vec<(i64, String)> = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let done = loop {
            let (_, j) = call_json(
                &app,
                Method::GET,
                &format!("/design/3d/jobs/{jid}"),
                None,
                false,
            )
            .await;
            if j["status"] != "queued" && j["status"] != "running" {
                break j;
            }
            let row = (
                j["progress_pct"].as_i64().unwrap(),
                j["stage"].as_str().unwrap().to_string(),
            );
            if seen.last() != Some(&row) {
                seen.push(row);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "job never finished: {seen:?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        assert_eq!(done["status"], "succeeded", "{done}");
        assert_eq!(done["progress_pct"], 100);
        for want in [
            (20, "Making game-ready with Otto Forge (local)"),
            (30, "Baking ORM maps"),
            (60, "Building LODs"),
            // A lower pct never moves the bar back; 100 waits for the commit.
            (60, "Packing collision"),
            (99, "wrote output"),
        ] {
            assert!(
                seen.iter().any(|(p, s)| *p == want.0 && s == want.1),
                "missing {want:?} in {seen:?}"
            );
        }
        assert!(
            seen.windows(2).all(|w| w[0].0 <= w[1].0),
            "progress moved backwards: {seen:?}"
        );

        let jid = start("rig").await;
        let failed = wait_job(&app, &jid).await;
        assert_eq!(failed["status"], "failed", "{failed}");
        let err = failed["error"].as_str().unwrap();
        assert!(err.contains("SkinTokens weights are missing"), "{err}");
        assert!(!err.contains("Traceback"), "{err}");
    }

    /// A kokoro provider asks for visemes (`X-Otto-Visemes: 1`); the clip keeps
    /// the timeline only when the server names one and serves a valid,
    /// small enough Rhubarb JSON for it.
    #[tokio::test]
    async fn kokoro_clip_stores_a_valid_viseme_timeline_only() {
        use axum::extract::Path;
        use axum::http::HeaderMap;
        use axum::response::{IntoResponse, Response};
        use axum::routing::{get, post};

        async fn speech(headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
            let voice = body["voice"].as_str().unwrap_or_default().to_string();
            let asked = headers.get("x-otto-visemes").is_some_and(|v| v == "1");
            let mut resp = ([("content-type", "audio/wav")], tiny_wav()).into_response();
            if asked && voice != "none" {
                resp.headers_mut()
                    .insert("x-otto-visemes-id", voice.parse().unwrap());
            }
            resp
        }
        async fn visemes(Path(id): Path<String>) -> Response {
            let cues = json!([
                { "start": 0.0, "end": 0.2, "value": "B" },
                { "start": 0.2, "end": 0.5, "value": "X" }
            ]);
            match id.as_str() {
                "good" => axum::Json(json!({ "duration_sec": 0.5, "cues": cues })).into_response(),
                "big" => axum::Json(
                    json!({ "duration_sec": 0.5, "cues": cues, "pad": "x".repeat(600 * 1024) }),
                )
                .into_response(),
                "bad" => axum::Json(json!({
                    "duration_sec": 0.5,
                    "cues": [{ "start": 0.0, "end": 0.2, "value": "Q" }]
                }))
                .into_response(),
                "text" => "not json".into_response(),
                _ => StatusCode::NOT_FOUND.into_response(),
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let mock = Router::new()
                .route("/v1/audio/speech", post(speech))
                .route("/v1/audio/visemes/{id}", get(visemes));
            axum::serve(listener, mock).await.unwrap();
        });

        let (ctx, ws_id, _dd, _ws, _sec) = setup_test(false).await;
        let app = api_router::<TestCtx>()
            .layer(Extension(AuthUser(test_user("u1"))))
            .with_state(ctx.clone());
        let (st, provider) = call_json(
            &app,
            Method::POST,
            "/product/tts/providers",
            Some(json!({
                "name": "Forge Voice",
                "kind": "kokoro",
                "endpoint_url": format!("http://127.0.0.1:{port}/"),
                "allow_local_network": true,
                "enabled": true
            })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{provider}");
        let pid = provider["id"].as_str().unwrap();

        for (voice, want_cues) in [
            ("good", 2),
            ("none", 0),
            ("gone", 0),
            ("big", 0),
            ("bad", 0),
            ("text", 0),
        ] {
            let (st, clip) = call_json(
                &app,
                Method::POST,
                "/product/tts/synthesize",
                Some(json!({
                    "workspace_id": ws_id,
                    "provider_id": pid,
                    "voice_id": voice,
                    "text": "Follow the lantern.",
                    "format": "wav"
                })),
                false,
            )
            .await;
            assert_eq!(st, StatusCode::CREATED, "{voice}: {clip}");
            // Read it back from the database, not the synthesize response.
            let (_, stored) = call_json(
                &app,
                Method::GET,
                &format!("/product/tts/clips/{}", clip["id"].as_str().unwrap()),
                None,
                false,
            )
            .await;
            if want_cues == 0 {
                assert!(stored["visemes"].is_null(), "{voice}: {stored}");
            } else {
                assert_eq!(stored["visemes"]["duration_sec"], 0.5, "{voice}: {stored}");
                let cues = stored["visemes"]["cues"].as_array().unwrap();
                assert_eq!(cues.len(), want_cues);
                assert_eq!(cues[0]["value"], "B");
                assert_eq!(cues[1]["end"], 0.5);
            }
        }
        server.abort();
    }

    fn app(ctx: &TestCtx) -> Router {
        api_router::<TestCtx>()
            .layer(Extension(AuthUser(test_user("u1"))))
            .with_state(ctx.clone())
    }

    async fn seed_glb(ctx: &TestCtx, ws_id: &str) -> String {
        ctx.design()
            .create_artifact(otto_design::service::CreateInput {
                workspace_id: ws_id.to_string(),
                project_id: None,
                studio: Some("3d".into()),
                format: "glb".into(),
                title: "Crate".into(),
                tags: vec![],
                meta: json!({}),
                content: Some(minimal_glb()),
                story_id: None,
                derived_from: None,
                created_by: "u1".into(),
                author: otto_design::service::Author::user("u1"),
                message: Some("initial".into()),
                source: None,
                created_at: None,
                version_kind: Some("named".into()),
                validate: true,
                provenance: None,
            })
            .await
            .unwrap()
            .artifact
            .id
    }

    async fn start_job(app: &Router, body: Value) -> (StatusCode, Value) {
        call_json(app, Method::POST, "/design/3d/jobs", Some(body), false).await
    }

    async fn get_json(app: &Router, uri: &str) -> Value {
        let (st, v) = call_json(app, Method::GET, uri, None, false).await;
        assert_eq!(st, StatusCode::OK, "{uri}: {v}");
        v
    }

    async fn wait_running(app: &Router, jid: &str) {
        for _ in 0..80 {
            if get_json(app, &format!("/design/3d/jobs/{jid}")).await["status"] == "running" {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("job {jid} never ran");
    }

    /// A clip's `artifact_id` names an artifact of the clip's own workspace.
    #[tokio::test]
    async fn tts_clip_artifact_must_belong_to_the_clip_workspace() {
        let (ctx, ws_id, _dd, _ws_dir, _secrets) = setup_test(true).await;
        let other_dir = tempfile::tempdir().unwrap();
        let other = ctx
            .workspaces
            .create(
                "Other",
                other_dir.path().to_str().unwrap(),
                &"u1".to_string(),
            )
            .await
            .unwrap();
        let own = seed_glb(&ctx, &ws_id).await;
        let foreign = seed_glb(&ctx, &other.id).await;
        for (artifact_id, want) in [
            (own.as_str(), StatusCode::CREATED),
            (foreign.as_str(), StatusCode::FORBIDDEN),
            ("missing", StatusCode::NOT_FOUND),
        ] {
            let (st, body) = call_json(
                &app(&ctx),
                Method::POST,
                "/product/tts/synthesize",
                Some(json!({
                    "workspace_id": ws_id,
                    "artifact_id": artifact_id,
                    "voice_id": "Samantha",
                    "text": "Hold the western gate."
                })),
                false,
            )
            .await;
            assert_eq!(st, want, "{artifact_id}: {body}");
        }
    }

    /// A 500 keeps its detail (host paths, SQLite, Keychain) server-side.
    #[tokio::test]
    async fn internal_error_body_keeps_its_detail_server_side() {
        let (ctx, ws_id, _dd, _ws_dir, _secrets) = setup_test(true).await;
        // A file where the clips directory goes: writing the clip fails.
        std::fs::create_dir_all(ctx.data_dir.join("media")).unwrap();
        std::fs::write(ctx.data_dir.join("media").join("tts"), b"").unwrap();
        let (st, body) = call_json(
            &app(&ctx),
            Method::POST,
            "/product/tts/synthesize",
            Some(json!({ "workspace_id": ws_id, "voice_id": "Samantha", "text": "Hold." })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
        assert_eq!(
            body,
            json!({ "code": "internal", "message": "internal server error" })
        );
    }

    /// A job cancelled while its provider finishes (a synchronous HTTP call
    /// never looks at the flag) saves no artifact and writes no export.
    #[tokio::test]
    async fn cancelled_job_commits_no_artifact_and_writes_no_export() {
        let (ctx, ws_id, _dd, ws_dir, _secrets) = setup_test(true).await;
        let app = app(&ctx);
        let (st, job) = start_job(
            &app,
            json!({
                "workspace_id": ws_id,
                "provider_id": BUILTIN_HUNYUAN_ID,
                "prompt": "finish after cancel",
                "export_relative_path": "assets/cancelled.glb"
            }),
        )
        .await;
        assert_eq!(st, StatusCode::ACCEPTED, "{job}");
        let jid = job["id"].as_str().unwrap();
        wait_running(&app, jid).await;
        let (st, cancelled) = call_json(
            &app,
            Method::POST,
            &format!("/design/3d/jobs/{jid}/cancel"),
            None,
            false,
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{cancelled}");
        // The driver returns within 25 ms of the cancel: give the job time to commit.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let artifacts: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM design_artifacts WHERE workspace_id = ?")
                .bind(&ws_id)
                .fetch_one(&ctx.pool)
                .await
                .unwrap();
        assert_eq!(artifacts, 0);
        assert!(!ws_dir.path().join("assets/cancelled.glb").exists());
        let done = get_json(&app, &format!("/design/3d/jobs/{jid}")).await;
        assert_eq!(done["status"], "cancelled", "{done}");
    }

    /// One job runs generate, rig and animate in a row: each stage gets the
    /// previous stage's mesh and commits its own version of the one artifact,
    /// and the job row names the chain and the stage it reached.
    #[tokio::test]
    async fn chained_job_runs_every_stage_and_commits_a_version_each() {
        let (ctx, ws_id, _dd, _ws_dir, _secrets) = setup_test(true).await;
        let app = app(&ctx);
        for bad in [
            json!(["generate"]),
            json!(["rig", "fly"]),
            json!(["rig", "rig", "rig", "rig"]),
        ] {
            let (st, v) = start_job(
                &app,
                json!({ "workspace_id": ws_id, "provider_id": BUILTIN_OTTO_FORGE_ID,
                        "prompt": "knight", "then_stages": bad }),
            )
            .await;
            assert_eq!(st, StatusCode::BAD_REQUEST, "{bad}: {v}");
        }
        let (st, job) = start_job(
            &app,
            json!({ "workspace_id": ws_id, "provider_id": BUILTIN_OTTO_FORGE_ID,
                    "prompt": "knight", "then_stages": ["rig", "animate"] }),
        )
        .await;
        assert_eq!(st, StatusCode::ACCEPTED, "{job}");
        assert_eq!(job["then_stages"], json!(["rig", "animate"]));
        let done = wait_job(&app, job["id"].as_str().unwrap()).await;
        assert_eq!(done["status"], "succeeded", "{done}");
        assert_eq!(done["pipeline_stage"], "animate", "{done}");
        assert_eq!(done["then_stages"], json!(["rig", "animate"]));
        assert_eq!(done["progress_pct"], 100);
        let aid = done["artifact_id"].as_str().unwrap();
        let messages: Vec<String> = sqlx::query_scalar(
            "SELECT message FROM design_versions WHERE artifact_id = ? ORDER BY seq",
        )
        .bind(aid)
        .fetch_all(&ctx.pool)
        .await
        .unwrap();
        assert_eq!(
            messages,
            [
                "Generated via Otto Forge (local)",
                "Stage 'rig' via Otto Forge (local)",
                "Stage 'animate' via Otto Forge (local)",
            ]
        );
        let last: String = sqlx::query_scalar(
            "SELECT provenance_json FROM design_versions WHERE artifact_id = ? ORDER BY seq DESC LIMIT 1",
        )
        .bind(aid)
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
        let last: Value = serde_json::from_str(&last).unwrap();
        assert_eq!(last["generator"]["pipeline_stage"], "animate", "{last}");
        assert_eq!(last["generator"]["source_artifact_id"], aid, "{last}");
    }

    #[test]
    fn chain_progress_spans_follow_stage_length() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            crate::service::stage_progress_spans(&s(&["rig"])),
            [(0, 100)]
        );
        assert_eq!(
            crate::service::stage_progress_spans(&s(&["generate", "rig", "animate"])),
            [(20, 77), (77, 91), (91, 95)]
        );
    }

    /// Boot recovery: a job a dead daemon left in flight fails as interrupted;
    /// a finished one is left alone.
    #[tokio::test]
    async fn restart_fails_the_jobs_a_dead_daemon_left_in_flight() {
        let (ctx, ws_id, _dd, _ws_dir, _secrets) = setup_test(true).await;
        let app = app(&ctx);
        let mut ids = Vec::new();
        for prompt in ["Crate", "finish after cancel"] {
            let (st, job) = start_job(
                &app,
                json!({ "workspace_id": ws_id, "provider_id": BUILTIN_HUNYUAN_ID, "prompt": prompt }),
            )
            .await;
            assert_eq!(st, StatusCode::ACCEPTED, "{job}");
            ids.push(job["id"].as_str().unwrap().to_string());
        }
        assert_eq!(wait_job(&app, &ids[0]).await["status"], "succeeded");
        wait_running(&app, &ids[1]).await;

        assert_eq!(
            ctx.media.store().fail_interrupted_3d_jobs().await.unwrap(),
            1
        );
        let done = get_json(&app, &format!("/design/3d/jobs/{}", ids[0])).await;
        assert_eq!(done["status"], "succeeded", "{done}");
        let live = get_json(&app, &format!("/design/3d/jobs/{}", ids[1])).await;
        assert_eq!(live["status"], "failed", "{live}");
        assert!(
            live["error"].as_str().unwrap().contains("restart"),
            "{live}"
        );
        assert!(live["finished_at"].is_string(), "{live}");
    }

    /// ElevenLabs answers `pcm_22050` with headerless PCM; the `.wav` clip
    /// Otto keeps is a RIFF file and knows its duration.
    #[tokio::test]
    async fn elevenlabs_wav_clip_is_a_riff_file_with_its_duration() {
        use axum::routing::post;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        // One second of 16-bit mono silence at 22.05 kHz.
        let mock = Router::new().route(
            "/v1/text-to-speech/{voice}",
            post(|| async { vec![0u8; 44_100] }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

        let (ctx, ws_id, _dd, _ws_dir, _secrets) = setup_test(false).await;
        let app = app(&ctx);
        let (st, provider) = call_json(
            &app,
            Method::POST,
            "/product/tts/providers",
            Some(json!({
                "name": "ElevenLabs",
                "kind": "elevenlabs",
                "endpoint_url": format!("http://127.0.0.1:{port}/"),
                "allow_local_network": true,
                "api_key": "xi-test"
            })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{provider}");
        let (st, clip) = call_json(
            &app,
            Method::POST,
            "/product/tts/synthesize",
            Some(json!({
                "workspace_id": ws_id,
                "provider_id": provider["id"],
                "voice_id": "Rachel",
                "text": "Follow the lantern.",
                "format": "wav"
            })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{clip}");
        assert_eq!(clip["mime"], "audio/wav");
        assert_eq!(clip["duration_ms"], 1000, "{clip}");
        let rec = ctx
            .media
            .store()
            .require_tts_clip(clip["id"].as_str().unwrap())
            .await
            .unwrap();
        let bytes = ctx.media.read_tts_clip_bytes(&rec).await.unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(bytes.len(), 44 + 44_100);
        assert_eq!(crate::tts::wav_duration_ms(&bytes), Some(1000));
        server.abort();
    }

    /// A local CLI runs by path, never looked up on `PATH`: a bare name saves
    /// a provider that can never run, so it is refused.
    #[tokio::test]
    async fn local_cli_provider_requires_an_absolute_cli_binary() {
        let (ctx, _ws_id, _dd, _ws_dir, _secrets) = setup_test(true).await;
        let app = app(&ctx);
        let create = |bin: &'static str| {
            json!({
                "name": "Otto Forge (local)",
                "kind": "custom_cli",
                "transport": "local_cli",
                "cli_binary": bin
            })
        };
        let (st, body) = call_json(
            &app,
            Method::POST,
            "/design/3d/providers",
            Some(create("forge")),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");
        let (st, body) = call_json(
            &app,
            Method::PATCH,
            &format!("/design/3d/providers/{BUILTIN_OTTO_FORGE_ID}"),
            Some(json!({ "cli_binary": "forge" })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");
        let (st, body) = call_json(
            &app,
            Method::POST,
            "/design/3d/providers",
            Some(create("/bin/echo")),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{body}");
        // Clearing it hands the built-in back to auto-detection.
        let (st, body) = call_json(
            &app,
            Method::PATCH,
            &format!("/design/3d/providers/{BUILTIN_OTTO_FORGE_ID}"),
            Some(json!({ "cli_binary": "" })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{body}");
    }

    /// A job's `export_relative_path` is checked before the run; a write that
    /// still fails is on the job (its artifact is saved), never silent.
    #[tokio::test]
    async fn job_export_path_is_checked_first_and_a_failed_write_is_reported() {
        let (ctx, ws_id, _dd, ws_dir, _secrets) = setup_test(true).await;
        let app = app(&ctx);
        let job = |export: &str| {
            json!({
                "workspace_id": ws_id,
                "provider_id": BUILTIN_HUNYUAN_ID,
                "prompt": "Crate",
                "export_relative_path": export
            })
        };
        for bad in ["../escape.glb", "notes.txt"] {
            let (st, body) = start_job(&app, job(bad)).await;
            assert_eq!(st, StatusCode::BAD_REQUEST, "{bad}: {body}");
        }
        // A file where the export's directory goes: the write fails at the end.
        std::fs::write(ws_dir.path().join("blocked"), b"").unwrap();
        let (st, started) = start_job(&app, job("blocked/model.glb")).await;
        assert_eq!(st, StatusCode::ACCEPTED, "{started}");
        let done = wait_job(&app, started["id"].as_str().unwrap()).await;
        assert_eq!(done["status"], "succeeded", "{done}");
        assert!(done["artifact_id"].is_string(), "{done}");
        assert!(done["exported_path"].is_null(), "{done}");
        assert!(
            done["error"].as_str().unwrap().contains("not a directory"),
            "{done}"
        );
    }

    /// A job asks only for a format its provider writes, and a target
    /// artifact's new version keeps the artifact's format.
    #[tokio::test]
    async fn job_output_format_must_suit_the_provider_and_the_target() {
        let (ctx, ws_id, _dd, _ws_dir, _secrets) = setup_test(true).await;
        let app = app(&ctx);
        let target = seed_glb(&ctx, &ws_id).await;
        for (provider, format, target_id, want) in [
            (BUILTIN_OTTO_FORGE_ID, "obj", None, StatusCode::BAD_REQUEST),
            (BUILTIN_HUNYUAN_ID, "gltf", None, StatusCode::BAD_REQUEST),
            (
                BUILTIN_HUNYUAN_ID,
                "obj",
                Some(&target),
                StatusCode::BAD_REQUEST,
            ),
            (BUILTIN_HUNYUAN_ID, "obj", None, StatusCode::ACCEPTED),
            (
                BUILTIN_HUNYUAN_ID,
                "glb",
                Some(&target),
                StatusCode::ACCEPTED,
            ),
        ] {
            let (st, body) = start_job(
                &app,
                json!({
                    "workspace_id": ws_id,
                    "provider_id": provider,
                    "prompt": "Crate",
                    "output_format": format,
                    "target_artifact_id": target_id
                }),
            )
            .await;
            assert_eq!(st, want, "{provider} {format} {target_id:?}: {body}");
        }
    }

    /// An unknown preset or a zero poly budget is refused, not dropped.
    #[tokio::test]
    async fn unknown_preset_and_zero_poly_budget_are_refused() {
        let (ctx, ws_id, _dd, _ws_dir, _secrets) = setup_test(true).await;
        let app = app(&ctx);
        for (extra, want) in [
            (json!({ "preset": "game_char" }), StatusCode::BAD_REQUEST),
            (json!({ "poly_budget": 0 }), StatusCode::BAD_REQUEST),
            (
                json!({ "preset": "stylized_character" }),
                StatusCode::ACCEPTED,
            ),
        ] {
            let mut body = json!({
                "workspace_id": ws_id,
                "provider_id": BUILTIN_HUNYUAN_ID,
                "prompt": "Crate"
            });
            body.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let (st, resp) = start_job(&app, body).await;
            assert_eq!(st, want, "{extra}: {resp}");
        }
    }

    /// `auth_header` is an HTTP header name: anything else is refused when
    /// saved rather than failing every later request.
    #[tokio::test]
    async fn provider_auth_header_must_be_a_header_name() {
        let (ctx, _ws_id, _dd, _ws_dir, _secrets) = setup_test(true).await;
        let app = app(&ctx);
        let tts =
            |hdr: &str| json!({ "name": "Voice", "kind": "openai_compat", "auth_header": hdr });
        let (st, body) = call_json(
            &app,
            Method::POST,
            "/product/tts/providers",
            Some(tts("X-Api Key")),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");
        let (st, created) = call_json(
            &app,
            Method::POST,
            "/product/tts/providers",
            Some(tts("X-Api-Key")),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{created}");
        let (st, body) = call_json(
            &app,
            Method::PATCH,
            &format!("/product/tts/providers/{}", created["id"].as_str().unwrap()),
            Some(json!({ "auth_header": "X-Api-Key:\r\n" })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");
        let (st, body) = call_json(
            &app,
            Method::POST,
            "/design/3d/providers",
            Some(json!({
                "name": "GPU Box",
                "kind": "custom_http",
                "transport": "http",
                "auth_header": "X Api"
            })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");
        let (st, body) = call_json(
            &app,
            Method::PATCH,
            &format!("/design/3d/providers/{BUILTIN_TRELLIS2_ID}"),
            Some(json!({ "auth_header": "X Api" })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{body}");
    }

    /// Grants a workspace's Viewer role and nothing above it.
    struct ViewerOnly;
    impl RoleChecker for ViewerOnly {
        fn check<'a>(
            &'a self,
            _user: &'a User,
            _workspace_id: &'a otto_core::Id,
            min: WorkspaceRole,
        ) -> BoxFuture<'a, Result<()>> {
            Box::pin(async move {
                if min <= WorkspaceRole::Viewer {
                    Ok(())
                } else {
                    Err(Error::Forbidden("viewer".into()))
                }
            })
        }
    }

    /// Each workspace gate asks for its route's role: a Viewer reads jobs and
    /// clips, and every write is forbidden.
    #[tokio::test]
    async fn viewer_reads_media_and_every_write_is_forbidden() {
        let (mut ctx, ws_id, _dd, _ws_dir, _secrets) = setup_test(true).await;
        let editor = app(&ctx);
        let (_, job) = start_job(
            &editor,
            json!({ "workspace_id": ws_id, "provider_id": BUILTIN_HUNYUAN_ID, "prompt": "Crate" }),
        )
        .await;
        let jid = job["id"].as_str().unwrap().to_string();
        let aid = wait_job(&editor, &jid).await["artifact_id"]
            .as_str()
            .unwrap()
            .to_string();
        let (_, clip) = call_json(
            &editor,
            Method::POST,
            "/product/tts/synthesize",
            Some(json!({ "workspace_id": ws_id, "voice_id": "Samantha", "text": "Hold." })),
            false,
        )
        .await;
        let cid = clip["id"].as_str().unwrap().to_string();

        ctx.roles = Arc::new(ViewerOnly);
        let viewer = app(&ctx);
        for uri in [
            format!("/design/3d/jobs?workspace_id={ws_id}"),
            format!("/design/3d/jobs/{jid}"),
            format!("/product/tts/clips?workspace_id={ws_id}"),
            format!("/product/tts/clips/{cid}"),
        ] {
            get_json(&viewer, &uri).await;
        }
        let export = json!({ "relative_path": "assets/x.glb" });
        for (method, uri, body) in [
            (
                Method::POST,
                "/design/3d/jobs".to_string(),
                Some(
                    json!({ "workspace_id": ws_id, "provider_id": BUILTIN_HUNYUAN_ID, "prompt": "Crate" }),
                ),
            ),
            (Method::POST, format!("/design/3d/jobs/{jid}/cancel"), None),
            (
                Method::POST,
                format!("/design/3d/artifacts/{aid}/export-workspace"),
                Some(export.clone()),
            ),
            (
                Method::POST,
                "/product/tts/synthesize".to_string(),
                Some(json!({ "workspace_id": ws_id, "voice_id": "Samantha", "text": "Hold." })),
            ),
            (
                Method::POST,
                format!("/product/tts/clips/{cid}/export-workspace"),
                Some(json!({ "relative_path": "assets/x.wav" })),
            ),
            (Method::DELETE, format!("/product/tts/clips/{cid}"), None),
        ] {
            let (st, resp) = call_json(&viewer, method.clone(), &uri, body, false).await;
            assert_eq!(st, StatusCode::FORBIDDEN, "{method} {uri}: {resp}");
        }
    }

    /// An async 3D provider's `status_url` gets the API key on the configured
    /// origin only; another origin is polled without it.
    #[tokio::test]
    async fn status_poll_sends_the_api_key_to_the_provider_origin_only() {
        use axum::http::HeaderMap;
        use axum::routing::{get, post};
        use base64::Engine;

        type Seen = Arc<Mutex<Vec<bool>>>;
        async fn status(
            axum::extract::State(seen): axum::extract::State<Seen>,
            headers: HeaderMap,
        ) -> axum::Json<Value> {
            seen.lock()
                .unwrap()
                .push(headers.contains_key("authorization"));
            let glb = base64::engine::general_purpose::STANDARD.encode(minimal_glb());
            axum::Json(json!({ "status": "succeeded", "glb_b64": glb }))
        }
        async fn serve(seen: Seen) -> u16 {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let mock = Router::new()
                // Answers with the status URL the job's params name.
                .route(
                    "/gen",
                    post(|axum::Json(body): axum::Json<Value>| async move {
                        axum::Json(json!({ "status_url": body["poll"] }))
                    }),
                )
                .route("/status", get(status))
                .with_state(seen);
            tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
            port
        }
        let (seen_a, seen_b): (Seen, Seen) = Default::default();
        let (port_a, port_b) = (serve(seen_a.clone()).await, serve(seen_b.clone()).await);

        let (ctx, ws_id, _dd, _ws_dir, _secrets) = setup_test(false).await;
        let app = app(&ctx);
        let (st, provider) = call_json(
            &app,
            Method::POST,
            "/design/3d/providers",
            Some(json!({
                "name": "GPU Box",
                "kind": "custom_http",
                "transport": "http",
                "endpoint_url": format!("http://127.0.0.1:{port_a}/gen"),
                "allow_local_network": true,
                "api_key": "secret-3d-key"
            })),
            false,
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{provider}");
        for port in [port_a, port_b] {
            let (st, job) = start_job(
                &app,
                json!({
                    "workspace_id": ws_id,
                    "provider_id": provider["id"],
                    "prompt": "Crate",
                    "params": { "poll": format!("http://127.0.0.1:{port}/status") }
                }),
            )
            .await;
            assert_eq!(st, StatusCode::ACCEPTED, "{job}");
            let mut done = Value::Null;
            for _ in 0..80 {
                done = get_json(
                    &app,
                    &format!("/design/3d/jobs/{}", job["id"].as_str().unwrap()),
                )
                .await;
                if done["status"] != "queued" && done["status"] != "running" {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            assert_eq!(done["status"], "succeeded", "{done}");
        }
        assert_eq!(*seen_a.lock().unwrap(), vec![true]);
        assert_eq!(*seen_b.lock().unwrap(), vec![false]);
    }

    /// A voice list that fails is an error and is not cached, so the next
    /// call reads the list once the server answers; a server with no voice
    /// list (404) gets the built-in voices.
    #[tokio::test]
    async fn failing_voice_list_is_an_error_and_not_cached() {
        use axum::response::IntoResponse;
        use axum::routing::get;
        use std::sync::atomic::Ordering;
        let up = Arc::new(AtomicBool::new(false));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let flag = up.clone();
        let mock = Router::new().route(
            "/v1/audio/voices",
            get(move || {
                let up = flag.load(Ordering::SeqCst);
                async move {
                    if up {
                        axum::Json(json!({ "voices": ["af_forge"] })).into_response()
                    } else {
                        StatusCode::INTERNAL_SERVER_ERROR.into_response()
                    }
                }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

        let (ctx, _ws_id, _dd, _ws_dir, _secrets) = setup_test(false).await;
        let app = app(&ctx);
        let mut ids = Vec::new();
        for path in ["", "nothing/"] {
            let (st, p) = call_json(
                &app,
                Method::POST,
                "/product/tts/providers",
                Some(json!({
                    "name": "Forge Voice",
                    "kind": "kokoro",
                    "endpoint_url": format!("http://127.0.0.1:{port}/{path}"),
                    "allow_local_network": true
                })),
                false,
            )
            .await;
            assert_eq!(st, StatusCode::CREATED, "{p}");
            ids.push(p["id"].as_str().unwrap().to_string());
        }
        let voices = |id: &str| format!("/product/tts/providers/{id}/voices");
        let (st, body) = call_json(&app, Method::GET, &voices(&ids[0]), None, false).await;
        assert_eq!(st, StatusCode::BAD_GATEWAY, "{body}");
        up.store(true, Ordering::SeqCst);
        assert_eq!(get_json(&app, &voices(&ids[0])).await[0]["id"], "af_forge");
        assert_eq!(get_json(&app, &voices(&ids[1])).await[0]["id"], "af_heart");
        server.abort();
    }

    mod audio_flows;
    mod production_flows;
}
