//! Extensible Custom HTTP & Custom Local CLI 3D generator & stage-pipeline
//! drivers (also powers built-in `otto-forge`; zero ComfyUI).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use async_trait::async_trait;
use base64::Engine;
use otto_core::{Error, Result};
use serde_json::json;

use super::{
    expand_cli_args, post_3d_http_and_collect, read_optional_manifest, run_cli_argv,
    test_3d_provider_connection, validate_3d_output, validate_cli_binary, Gen3dDriver, Gen3dOutput,
    Gen3dRequest,
};
use crate::types::{Media3dProviderRecord, ProviderTestResult};

pub struct CustomHttpDriver;
pub struct CustomCliDriver;

#[async_trait]
impl Gen3dDriver for CustomHttpDriver {
    async fn generate(
        &self,
        req: &Gen3dRequest,
        provider: &Media3dProviderRecord,
        api_key: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<Gen3dOutput> {
        let mut body = provider
            .default_params
            .as_object()
            .cloned()
            .unwrap_or_default();
        if let Some(extra) = req.params.as_object() {
            for (k, v) in extra {
                body.insert(k.clone(), v.clone());
            }
        }
        body.insert("stage".into(), json!(req.pipeline_stage));
        if let Some(prompt) = req.prompt.as_deref().filter(|s| !s.trim().is_empty()) {
            body.insert("prompt".into(), json!(prompt));
        }
        if let Some(img) = &req.image_bytes {
            body.insert(
                "image_b64".into(),
                json!(base64::engine::general_purpose::STANDARD.encode(img)),
            );
        }
        if let Some(mesh) = &req.source_mesh_bytes {
            body.insert(
                "mesh_b64".into(),
                json!(base64::engine::general_purpose::STANDARD.encode(mesh)),
            );
            if let Some(fmt) = &req.source_mesh_format {
                body.insert("mesh_format".into(), json!(fmt));
            }
        }
        body.insert("output_format".into(), json!(req.output_format));
        if let Some(seed) = req.seed {
            body.insert("seed".into(), json!(seed));
        }
        if let Some(budget) = req.poly_budget {
            body.insert("poly_budget".into(), json!(budget));
        }
        post_3d_http_and_collect(
            provider,
            api_key,
            &serde_json::Value::Object(body),
            &req.output_format,
            req.max_output_bytes,
            req.timeout,
            cancel,
        )
        .await
    }

    async fn test_connection(
        &self,
        provider: &Media3dProviderRecord,
        api_key: Option<&str>,
    ) -> Result<ProviderTestResult> {
        test_3d_provider_connection(provider, api_key).await
    }
}

#[async_trait]
impl Gen3dDriver for CustomCliDriver {
    async fn generate(
        &self,
        req: &Gen3dRequest,
        provider: &Media3dProviderRecord,
        _api_key: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<Gen3dOutput> {
        let bin_str = provider.effective_cli_binary().ok_or_else(|| {
            Error::Invalid(
                "local CLI provider requires cli_binary (or installed Otto Forge)".into(),
            )
        })?;
        let bin = validate_cli_binary(&bin_str)?;

        let tmp = tempfile::tempdir()
            .map_err(|e| Error::Internal(format!("create custom CLI tempdir: {e}")))?;
        // Named by the job: Otto Forge names its work folder after the output
        // (`out/work/<job>-…`), where Smart mode finds the concept picture.
        let out_file = tmp
            .path()
            .join(format!("{}.{}", req.job_id, req.output_format));
        let img_path = if let Some(img) = &req.image_bytes {
            let p = tmp.path().join("input.png");
            tokio::fs::write(&p, img)
                .await
                .map_err(|e| Error::Internal(format!("write reference image: {e}")))?;
            Some(p)
        } else {
            None
        };
        let mesh_path = if let Some(mesh) = &req.source_mesh_bytes {
            let ext = req
                .source_mesh_format
                .as_deref()
                .filter(|s| matches!(*s, "glb" | "gltf" | "obj"))
                .unwrap_or("glb");
            let p = tmp.path().join(format!("source.{ext}"));
            tokio::fs::write(&p, mesh)
                .await
                .map_err(|e| Error::Internal(format!("write source mesh: {e}")))?;
            Some(p)
        } else {
            None
        };

        let mut vars: HashMap<&str, String> = HashMap::new();
        vars.insert("stage", req.pipeline_stage.clone());
        vars.insert("prompt", req.prompt.clone().unwrap_or_default());
        vars.insert(
            "image",
            img_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
        );
        vars.insert(
            "mesh",
            mesh_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
        );
        vars.insert("output", out_file.to_string_lossy().to_string());
        vars.insert("output_dir", tmp.path().to_string_lossy().to_string());
        vars.insert("format", req.output_format.clone());
        vars.insert(
            "seed",
            req.seed
                .map(|s| s.to_string())
                .unwrap_or_else(|| "42".into()),
        );
        // The moves to make (`forge animate --clips a,b,c`); empty: the CLI's default.
        vars.insert(
            "clips",
            req.params
                .get("clips")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        );
        vars.insert(
            "poly_budget",
            req.poly_budget
                .map(|b| b.to_string())
                .unwrap_or_else(|| "25000".into()),
        );

        let mut args = expand_cli_args(&provider.cli_args_template, &vars);
        if req.pipeline_stage == "rig" && req.params.get("swimming_fish") == Some(&json!(true)) {
            args.push("--swimming-fish".into());
        }
        // A real size in metres (`forge generate --height-m 1.8`), sent only by
        // a caller that knows the CLI takes it (Smart mode asks Forge first).
        for (key, flag) in [
            ("height_m", "--height-m"),
            ("longest_side_m", "--longest-side-m"),
        ] {
            if let Some(m) = req.params.get(key).and_then(serde_json::Value::as_f64) {
                args.extend([flag.to_string(), m.to_string()]);
            }
        }
        // A first look without the game-ready pass (`forge generate
        // --skip-gameready`), from a caller that knows the CLI takes it.
        if req.params.get("skip_gameready") == Some(&json!(true)) {
            args.push("--skip-gameready".into());
        }
        if let Some(kind @ ("character" | "prop")) =
            req.params.get("kind").and_then(serde_json::Value::as_str)
        {
            args.extend(["--kind".to_string(), kind.to_string()]);
        }
        if let Some(body @ ("humanoid" | "quadruped" | "bird" | "legless" | "other")) =
            req.params.get("body").and_then(serde_json::Value::as_str)
        {
            args.extend(["--body".to_string(), body.to_string()]);
        }
        // A character draft's finishing inputs (`<draft>.glb.finish/`, named
        // in its manifest) are kept in `keep_dir` between its generate and its
        // game-ready pass, which gets them back beside its source mesh.
        let keep = req
            .params
            .get("keep_dir")
            .and_then(serde_json::Value::as_str)
            .and_then(kept_inputs_dir);
        if let (Some(keep), Some(mesh)) = (&keep, &mesh_path) {
            let inputs = std::path::PathBuf::from(format!("{}.finish", mesh.display()));
            if copy_dir(&keep.join("finish"), &inputs).await.is_ok() {
                let kept = tokio::fs::read(keep.join("manifest.json")).await.ok();
                let mut manifest: serde_json::Value = kept
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or_default();
                let name = inputs.file_name().unwrap_or_default().to_string_lossy();
                manifest["finish"]["inputs"] = json!(name);
                let sidecar = format!("{}.manifest.json", mesh.display());
                let _ = tokio::fs::write(sidecar, manifest.to_string()).await;
            }
        }
        let workdir = provider
            .cli_workdir
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(Path::new);
        run_cli_argv(
            &bin,
            &args,
            workdir,
            req.timeout,
            cancel,
            req.progress.as_ref(),
        )
        .await?;

        let bytes = tokio::fs::read(&out_file).await.map_err(|e| {
            Error::Upstream(format!(
                "custom 3D CLI succeeded but did not write '{}': {e}",
                out_file.display()
            ))
        })?;
        validate_3d_output(&req.output_format, &bytes, req.max_output_bytes)?;
        let manifest = read_optional_manifest(&out_file).await;
        if let (Some(keep), None) = (&keep, &mesh_path) {
            let inputs = std::path::PathBuf::from(format!("{}.finish", out_file.display()));
            if tokio::fs::metadata(&inputs).await.is_ok_and(|m| m.is_dir()) {
                let _ = tokio::fs::remove_dir_all(keep).await;
                copy_dir(&inputs, &keep.join("finish")).await.map_err(|e| {
                    Error::Internal(format!("keep the draft's finishing inputs: {e}"))
                })?;
                let kept = manifest.clone().unwrap_or_default().to_string();
                tokio::fs::write(keep.join("manifest.json"), kept)
                    .await
                    .map_err(|e| Error::Internal(format!("keep the draft's manifest: {e}")))?;
            }
        }
        Ok(Gen3dOutput {
            bytes,
            format: req.output_format.clone(),
            manifest,
        })
    }

    async fn test_connection(
        &self,
        provider: &Media3dProviderRecord,
        api_key: Option<&str>,
    ) -> Result<ProviderTestResult> {
        test_3d_provider_connection(provider, api_key).await
    }
}

/// A `keep_dir` Otto's productions use (`…/media/productions/<id>/<item>.finish`,
/// absolute, no `..`); anything else is ignored, never written or removed.
fn kept_inputs_dir(path: &str) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let p = std::path::PathBuf::from(path);
    let parts: Vec<Component> = p.components().collect();
    let ok = p.is_absolute()
        && !parts
            .iter()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        && p.extension().is_some_and(|e| e == "finish")
        && parts.windows(2).any(|w| {
            w[0] == Component::Normal("media".as_ref())
                && w[1] == Component::Normal("productions".as_ref())
        });
    ok.then_some(p)
}

/// Copy the folder `from` (and everything in it) to `to`.
async fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut dirs = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((src, dst)) = dirs.pop() {
        tokio::fs::create_dir_all(&dst).await?;
        let mut entries = tokio::fs::read_dir(&src).await?;
        while let Some(e) = entries.next_entry().await? {
            let target = dst.join(e.file_name());
            match e.file_type().await?.is_dir() {
                true => dirs.push((e.path(), target)),
                false => {
                    tokio::fs::copy(e.path(), target).await?;
                }
            }
        }
    }
    Ok(())
}
