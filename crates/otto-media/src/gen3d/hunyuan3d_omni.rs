//! Tencent Hunyuan3D-Omni adapter (`Tencent-Hunyuan/Hunyuan3D-Omni`).
//!
//! Real multi-modal conditioning inputs:
//! - Primary reference image (`image_b64`) and/or prompt
//! - `--control_type <none|point|voxel|bbox|pose>`
//!   - `point`: `.ply` point cloud file
//!   - `voxel`: voxel grid input
//!   - `bbox`: 3D bounding box `[w, h, d]` / `[min_x, min_y, min_z, max_x, max_y, max_z]`
//!   - `pose`: skeletal pose JSON / prior
//! - `--use_ema` (EMA weights) and `--flashvdm` (FlashVDM acceleration)
//! - Outputs `.glb` or `.obj`

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
use crate::types::{Media3dProviderRecord, ProviderTestResult, MEDIA_3D_CONTROLS};

pub struct Hunyuan3dOmniDriver;

fn merge_param<'a>(req: &'a Value, defaults: &'a Value, key: &str) -> Option<&'a Value> {
    req.get(key).or_else(|| defaults.get(key))
}

pub fn build_hunyuan_http_payload(
    req: &Gen3dRequest,
    provider: &Media3dProviderRecord,
) -> Result<Value> {
    let control_type = merge_param(&req.params, &provider.default_params, "control_type")
        .and_then(|v| v.as_str())
        .unwrap_or("none")
        .to_ascii_lowercase();
    if !MEDIA_3D_CONTROLS.contains(&control_type.as_str()) {
        return Err(Error::Invalid(format!(
            "Hunyuan3D-Omni control_type must be one of: {}",
            MEDIA_3D_CONTROLS.join(", ")
        )));
    }
    let use_ema = merge_param(&req.params, &provider.default_params, "use_ema")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let flashvdm = merge_param(&req.params, &provider.default_params, "flashvdm")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let steps = merge_param(&req.params, &provider.default_params, "steps")
        .and_then(|v| v.as_u64())
        .unwrap_or(30)
        .clamp(1, 200);
    let guidance_scale = merge_param(&req.params, &provider.default_params, "guidance_scale")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite())
        .unwrap_or(5.5)
        .clamp(0.5, 30.0);

    let mut body = json!({
        "model": "hunyuan3d-omni",
        "control_type": control_type,
        "use_ema": use_ema,
        "flashvdm": flashvdm,
        "steps": steps,
        "guidance_scale": guidance_scale,
        "output_format": req.output_format,
    });
    let obj = body.as_object_mut().expect("json object");
    if let Some(prompt) = req.prompt.as_deref().filter(|s| !s.trim().is_empty()) {
        obj.insert("prompt".into(), json!(prompt));
    }
    if let Some(img) = &req.image_bytes {
        obj.insert(
            "image".into(),
            json!(base64::engine::general_purpose::STANDARD.encode(img)),
        );
    }
    if let Some(seed) = req.seed {
        obj.insert("seed".into(), json!(seed));
    }
    if let Some(budget) = req.poly_budget {
        obj.insert("face_count".into(), json!(budget));
    }
    if let Some(bbox) = req.params.get("bbox") {
        obj.insert("bbox".into(), bbox.clone());
    }
    if let Some(ctrl_b64) = req.params.get("control_input_b64").and_then(|v| v.as_str()) {
        obj.insert("control_input_b64".into(), json!(ctrl_b64));
    }
    if let Some(ctrl_name) = req.params.get("control_filename").and_then(|v| v.as_str()) {
        obj.insert("control_filename".into(), json!(ctrl_name));
    }
    Ok(body)
}

#[async_trait]
impl Gen3dDriver for Hunyuan3dOmniDriver {
    async fn generate(
        &self,
        req: &Gen3dRequest,
        provider: &Media3dProviderRecord,
        api_key: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<Gen3dOutput> {
        match provider.transport.as_str() {
            "http" => {
                let payload = build_hunyuan_http_payload(req, provider)?;
                post_3d_http_and_collect(
                    provider,
                    api_key,
                    &payload,
                    &req.output_format,
                    req.max_output_bytes,
                    req.timeout,
                    cancel,
                )
                .await
            }
            "local_cli" => run_hunyuan_local_cli(req, provider, cancel).await,
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

async fn run_hunyuan_local_cli(
    req: &Gen3dRequest,
    provider: &Media3dProviderRecord,
    cancel: &AtomicBool,
) -> Result<Gen3dOutput> {
    let bin_str = provider
        .cli_binary
        .as_deref()
        .ok_or_else(|| Error::Invalid("Hunyuan3D-Omni local_cli requires cli_binary".into()))?;
    let bin = validate_cli_binary(bin_str)?;

    let tmp =
        tempfile::tempdir().map_err(|e| Error::Internal(format!("create 3D job tempdir: {e}")))?;
    let out_file = tmp.path().join(format!("output.{}", req.output_format));

    let img_path = if let Some(img) = &req.image_bytes {
        let ext = match req.image_mime.as_deref() {
            Some("image/jpeg") => "jpg",
            Some("image/webp") => "webp",
            _ => "png",
        };
        let p = tmp.path().join(format!("input.{ext}"));
        tokio::fs::write(&p, img)
            .await
            .map_err(|e| Error::Internal(format!("write reference image: {e}")))?;
        Some(p)
    } else {
        None
    };

    let control_type = merge_param(&req.params, &provider.default_params, "control_type")
        .and_then(|v| v.as_str())
        .unwrap_or("none")
        .to_ascii_lowercase();
    if !MEDIA_3D_CONTROLS.contains(&control_type.as_str()) {
        return Err(Error::Invalid(format!(
            "Hunyuan3D-Omni control_type must be one of: {}",
            MEDIA_3D_CONTROLS.join(", ")
        )));
    }

    let control_file_path =
        if let Some(ctrl_b64) = req.params.get("control_input_b64").and_then(|v| v.as_str()) {
            let clean = ctrl_b64
                .split_once(',')
                .map(|(_, rest)| rest)
                .unwrap_or(ctrl_b64);
            let raw = base64::engine::general_purpose::STANDARD
                .decode(clean.trim())
                .map_err(|e| Error::Invalid(format!("invalid control_input_b64: {e}")))?;
            let ext = match control_type.as_str() {
                "point" => "ply",
                "bbox" | "pose" => "json",
                _ => "bin",
            };
            let p = tmp.path().join(format!("control.{ext}"));
            tokio::fs::write(&p, &raw)
                .await
                .map_err(|e| Error::Internal(format!("write control input: {e}")))?;
            Some(p)
        } else if let Some(bbox) = req.params.get("bbox") {
            let p = tmp.path().join("control.json");
            let bytes = serde_json::to_vec(bbox)
                .map_err(|e| Error::Invalid(format!("invalid bbox JSON: {e}")))?;
            tokio::fs::write(&p, &bytes)
                .await
                .map_err(|e| Error::Internal(format!("write bbox file: {e}")))?;
            Some(p)
        } else {
            None
        };

    let mut vars: HashMap<&str, String> = HashMap::new();
    vars.insert("prompt", req.prompt.clone().unwrap_or_default());
    vars.insert(
        "image",
        img_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
    );
    vars.insert("output", out_file.to_string_lossy().to_string());
    vars.insert("output_dir", tmp.path().to_string_lossy().to_string());
    vars.insert("format", req.output_format.clone());
    vars.insert("control_type", control_type.clone());
    vars.insert(
        "control_file",
        control_file_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
    );
    vars.insert(
        "seed",
        req.seed
            .map(|s| s.to_string())
            .unwrap_or_else(|| "42".into()),
    );
    vars.insert(
        "poly_budget",
        req.poly_budget
            .map(|b| b.to_string())
            .unwrap_or_else(|| "50000".into()),
    );

    let args = if !provider.cli_args_template.is_empty() {
        expand_cli_args(&provider.cli_args_template, &vars)
    } else {
        let mut a = vec!["inference.py".to_string()];
        if control_type != "none" {
            a.push("--control_type".into());
            a.push(control_type);
        }
        if merge_param(&req.params, &provider.default_params, "use_ema")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
        {
            a.push("--use_ema".into());
        }
        if merge_param(&req.params, &provider.default_params, "flashvdm")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
        {
            a.push("--flashvdm".into());
        }
        if let Some(p) = &img_path {
            a.push("--image".into());
            a.push(p.to_string_lossy().to_string());
        }
        if let Some(cp) = &control_file_path {
            a.push("--control_input".into());
            a.push(cp.to_string_lossy().to_string());
        }
        a.push("--output".into());
        a.push(out_file.to_string_lossy().to_string());
        a
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
            "Hunyuan3D-Omni CLI succeeded but did not produce '{}': {e}",
            out_file.display()
        ))
    })?;
    validate_3d_output(&req.output_format, &bytes, req.max_output_bytes)?;
    let manifest = read_optional_manifest(&out_file).await;
    Ok(Gen3dOutput {
        bytes,
        format: req.output_format.clone(),
        manifest,
    })
}
