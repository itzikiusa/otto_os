//! Microsoft TRELLIS.2 adapter (`microsoft/TRELLIS.2` 4B O-Voxel PBR pipeline).
//!
//! Real pipeline & `o_voxel.postprocess.to_glb()` parameters:
//! - Primary input: clean/RGBA reference image (`image_b64`)
//! - `resolution`: `512` | `1024` | `1536` (sparse O-Voxel grid resolution)
//! - `steps`: flow-matching sampling steps (default `12`)
//! - `guidance_scale`: CFG strength (default `7.5`)
//! - `seed`: deterministic seed
//! - `decimation_target`: target triangle/face count (game polygon budget)
//! - `texture_size`: baked PBR texture resolution (`1024` | `2048` | `4096`)
//! - `remesh`: boolean
//! - Full PBR channels (Base Color, Roughness, Metallic, Opacity/Alpha) → `.glb`

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use async_trait::async_trait;
use base64::Engine;
use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::{
    expand_cli_args, post_3d_http_and_collect, read_optional_manifest, run_cli_argv,
    test_3d_provider_connection, validate_3d_output, validate_cli_binary, Gen3dDriver, Gen3dOutput,
    Gen3dRequest,
};
use crate::types::{Media3dProviderRecord, ProviderTestResult};

pub struct Trellis2Driver;

fn merge_param<'a>(req: &'a Value, defaults: &'a Value, key: &str) -> Option<&'a Value> {
    req.get(key).or_else(|| defaults.get(key))
}

pub fn build_trellis2_http_payload(
    req: &Gen3dRequest,
    provider: &Media3dProviderRecord,
) -> Result<Value> {
    let resolution = merge_param(&req.params, &provider.default_params, "resolution")
        .and_then(|v| v.as_u64())
        .unwrap_or(1024);
    if !matches!(resolution, 512 | 1024 | 1536) {
        return Err(Error::Invalid(
            "TRELLIS.2 resolution must be 512, 1024, or 1536".into(),
        ));
    }
    let texture_size = merge_param(&req.params, &provider.default_params, "texture_size")
        .and_then(|v| v.as_u64())
        .unwrap_or(2048);
    if !matches!(texture_size, 512 | 1024 | 2048 | 4096) {
        return Err(Error::Invalid(
            "TRELLIS.2 texture_size must be 512, 1024, 2048, or 4096".into(),
        ));
    }
    let steps = merge_param(&req.params, &provider.default_params, "steps")
        .and_then(|v| v.as_u64())
        .unwrap_or(12)
        .clamp(1, 100);
    let guidance_scale = merge_param(&req.params, &provider.default_params, "guidance_scale")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite())
        .unwrap_or(7.5)
        .clamp(0.5, 30.0);
    let decimation_target = req
        .poly_budget
        .map(u64::from)
        .or_else(|| {
            merge_param(&req.params, &provider.default_params, "decimation_target")
                .and_then(|v| v.as_u64())
        })
        .unwrap_or(100_000)
        .clamp(500, 2_000_000);
    let remesh = merge_param(&req.params, &provider.default_params, "remesh")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let pbr = merge_param(&req.params, &provider.default_params, "pbr")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let mut body = json!({
        "model": "trellis-2-4b",
        "resolution": resolution,
        "steps": steps,
        "guidance_scale": guidance_scale,
        "decimation_target": decimation_target,
        "texture_size": texture_size,
        "remesh": remesh,
        "pbr": pbr,
        "output_format": "glb",
    });
    let obj = body.as_object_mut().expect("json object");
    if let Some(img) = &req.image_bytes {
        obj.insert(
            "image".into(),
            json!(base64::engine::general_purpose::STANDARD.encode(img)),
        );
    }
    if let Some(prompt) = req.prompt.as_deref().filter(|s| !s.trim().is_empty()) {
        obj.insert("prompt".into(), json!(prompt));
    }
    if let Some(seed) = req.seed {
        obj.insert("seed".into(), json!(seed));
    }
    Ok(body)
}

#[async_trait]
impl Gen3dDriver for Trellis2Driver {
    async fn generate(
        &self,
        req: &Gen3dRequest,
        provider: &Media3dProviderRecord,
        api_key: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<Gen3dOutput> {
        match provider.transport.as_str() {
            "http" => {
                let payload = build_trellis2_http_payload(req, provider)?;
                post_3d_http_and_collect(
                    provider,
                    api_key,
                    &payload,
                    "glb",
                    req.max_output_bytes,
                    req.timeout,
                    cancel,
                )
                .await
            }
            "local_cli" => run_trellis2_local_cli(req, provider, cancel).await,
            other => Err(Error::Invalid(format!("unsupported transport '{other}'"))),
        }
    }

    async fn test_connection(
        &self,
        provider: &Media3dProviderRecord,
        api_key: Option<&str>,
    ) -> Result<ProviderTestResult> {
        test_3d_provider_connection(provider, api_key).await
    }
}

async fn run_trellis2_local_cli(
    req: &Gen3dRequest,
    provider: &Media3dProviderRecord,
    cancel: &AtomicBool,
) -> Result<Gen3dOutput> {
    let bin_str = provider
        .cli_binary
        .as_deref()
        .ok_or_else(|| Error::Invalid("TRELLIS.2 local_cli requires cli_binary".into()))?;
    let bin = validate_cli_binary(bin_str)?;

    let img = req.image_bytes.as_ref().ok_or_else(|| {
        Error::Invalid(
            "TRELLIS.2 requires a reference image (image_b64 or image_artifact_id)".into(),
        )
    })?;

    let tmp = tempfile::tempdir()
        .map_err(|e| Error::Internal(format!("create TRELLIS.2 job tempdir: {e}")))?;
    let img_path = tmp.path().join("input.png");
    tokio::fs::write(&img_path, img)
        .await
        .map_err(|e| Error::Internal(format!("write reference image: {e}")))?;
    let out_file = tmp.path().join("output.glb");

    let payload = build_trellis2_http_payload(req, provider)?;
    let resolution = payload["resolution"].as_u64().unwrap_or(1024).to_string();
    let decimation = payload["decimation_target"]
        .as_u64()
        .unwrap_or(100_000)
        .to_string();
    let tex_size = payload["texture_size"].as_u64().unwrap_or(2048).to_string();
    let steps = payload["steps"].as_u64().unwrap_or(12).to_string();

    let mut vars: HashMap<&str, String> = HashMap::new();
    vars.insert("image", img_path.to_string_lossy().to_string());
    vars.insert("output", out_file.to_string_lossy().to_string());
    vars.insert("output_dir", tmp.path().to_string_lossy().to_string());
    vars.insert("resolution", resolution.clone());
    vars.insert("poly_budget", decimation.clone());
    vars.insert("decimation_target", decimation.clone());
    vars.insert("texture_size", tex_size.clone());
    vars.insert("steps", steps.clone());
    vars.insert(
        "seed",
        req.seed
            .map(|s| s.to_string())
            .unwrap_or_else(|| "42".into()),
    );
    vars.insert("prompt", req.prompt.clone().unwrap_or_default());

    let args = if !provider.cli_args_template.is_empty() {
        expand_cli_args(&provider.cli_args_template, &vars)
    } else {
        vec![
            "infer.py".into(),
            "--image".into(),
            img_path.to_string_lossy().to_string(),
            "--output".into(),
            out_file.to_string_lossy().to_string(),
            "--resolution".into(),
            resolution,
            "--decimation-target".into(),
            decimation,
            "--texture-size".into(),
            tex_size,
            "--steps".into(),
            steps,
        ]
    };

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
            "TRELLIS.2 CLI succeeded but did not produce '{}': {e}",
            out_file.display()
        ))
    })?;
    validate_3d_output("glb", &bytes, req.max_output_bytes)?;
    let manifest = read_optional_manifest(&out_file).await;
    Ok(Gen3dOutput {
        bytes,
        format: "glb".into(),
        manifest,
    })
}
