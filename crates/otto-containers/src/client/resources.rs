//! Docker images, volumes, networks, and system prune operations.

use std::collections::BTreeSet;

use hyper::Method;
use otto_core::{Error, Result};
use serde_json::Value;

use crate::types::{
    ContainerActionResp, ImageInspectResp, ImagesListResp, NetworkSummary, NetworksListResp,
    PruneResp, PullImageReq, PullImageResp, SystemPruneReq, SystemPruneResp, VolumesListResp,
};

use super::containers::{validate_id, validate_image_ref};
use super::parse::{
    parse_image_inspect, parse_image_summary, parse_network_summary, parse_volume_summary,
    string_array, u64_field,
};
use super::transport::{
    check_status, encode_query_component, DEFAULT_TIMEOUT, MAX_JSON_BODY, PULL_TIMEOUT,
};
use super::{DockerClient, API_PREFIX};

impl DockerClient {
    /// List local images (`GET /containers/images`).
    pub async fn list_images(&self) -> Result<ImagesListResp> {
        let images_path = format!("{API_PREFIX}/images/json");
        let (raw_res, containers_res) = tokio::join!(
            self.get_json::<Vec<Value>>(&images_path, true, DEFAULT_TIMEOUT),
            self.list_containers(true),
        );
        let raw = raw_res?;
        let used_images: BTreeSet<String> = match containers_res {
            Ok(list) => list.containers.into_iter().map(|c| c.image_id).collect(),
            Err(err) => {
                tracing::warn!(
                    socket = %self.socket_path.display(),
                    error = %err,
                    "failed to list containers while resolving image usage"
                );
                return Err(err);
            }
        };

        let mut images = Vec::with_capacity(raw.len());
        for item in &raw {
            images.push(parse_image_summary(item, &used_images));
        }
        images.sort_by_key(|a| std::cmp::Reverse(a.created));
        Ok(ImagesListResp { images })
    }

    /// Inspect one local image (`GET /containers/images/{id}`): only the ports it
    /// declares. `/`, `:` and `@` stay literal, as the docker CLI sends them.
    pub async fn inspect_image(&self, reference: &str) -> Result<ImageInspectResp> {
        let reference = reference.trim();
        validate_image_ref(reference)?;
        let name = encode_query_component(reference)
            .replace("%2F", "/")
            .replace("%3A", ":")
            .replace("%40", "@");
        let v: Value = self
            .get_json(
                &format!("{API_PREFIX}/images/{name}/json"),
                true,
                DEFAULT_TIMEOUT,
            )
            .await?;
        Ok(parse_image_inspect(&v))
    }

    /// Pull an image from a registry (`POST /containers/images/pull`), streaming
    /// NDJSON progress frames incrementally so large pulls never falsely succeed
    /// on body truncation.
    pub async fn pull_image(&self, req: &PullImageReq) -> Result<PullImageResp> {
        let reference = req.reference.trim();
        validate_image_ref(reference)?;
        let (from_image, tag) = split_image_ref(reference);
        let mut path = format!(
            "{API_PREFIX}/images/create?fromImage={}",
            encode_query_component(from_image)
        );
        if let Some(t) = tag {
            path.push_str(&format!("&tag={}", encode_query_component(t)));
        }
        if let Some(p) = req
            .platform
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            path.push_str(&format!("&platform={}", encode_query_component(p)));
        }

        let mut events = Vec::new();
        let mut digest = None;
        let mut last_status = "Pulled".to_string();

        self.stream_ndjson(Method::POST, &path, false, PULL_TIMEOUT, |line| {
            if let Ok(val) = serde_json::from_str::<Value>(line) {
                if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                    return Err(Error::Upstream(format!("image pull failed: {err}")));
                }
                if let Some(status) = val.get("status").and_then(|s| s.as_str()) {
                    last_status = status.to_string();
                    if let Some(d) = status.strip_prefix("Digest: ") {
                        digest = Some(d.trim().to_string());
                    }
                    if events.len() < 50 {
                        events.push(status.to_string());
                    }
                }
            }
            Ok(())
        })
        .await?;

        Ok(PullImageResp {
            reference: reference.to_string(),
            status: last_status,
            digest,
            events,
        })
    }

    /// Delete an image by ID (`DELETE /containers/images/{id}`).
    pub async fn remove_image(&self, id: &str, force: bool) -> Result<ContainerActionResp> {
        validate_id("image id", id)?;
        let path = format!(
            "{API_PREFIX}/images/{}?force={force}",
            encode_query_component(id)
        );
        let resp = self
            .request(
                Method::DELETE,
                &path,
                None,
                false,
                DEFAULT_TIMEOUT,
                MAX_JSON_BODY,
            )
            .await?;
        check_status(&resp)?;
        Ok(ContainerActionResp {
            ok: true,
            id: id.to_string(),
            action: "remove_image".into(),
            message: None,
        })
    }

    /// Prune unused or dangling images (`POST /containers/images/prune`).
    pub async fn prune_images(&self, dangling_only: bool) -> Result<PruneResp> {
        let filters = if dangling_only {
            r#"{"dangling":["true"]}"#
        } else {
            r#"{"dangling":["false"]}"#
        };
        let path = format!(
            "{API_PREFIX}/images/prune?filters={}",
            encode_query_component(filters)
        );
        let resp = self
            .request(
                Method::POST,
                &path,
                None,
                false,
                DEFAULT_TIMEOUT,
                MAX_JSON_BODY,
            )
            .await?;
        check_status(&resp)?;
        let v: Value = serde_json::from_slice(&resp.body).unwrap_or(Value::Null);
        let deleted = v
            .get("ImagesDeleted")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        item.get("Deleted")
                            .or_else(|| item.get("Untagged"))
                            .and_then(|s| s.as_str())
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(PruneResp {
            kind: "images".into(),
            deleted,
            space_reclaimed_bytes: u64_field(&v, "SpaceReclaimed"),
        })
    }

    /// List volumes (`GET /containers/volumes`).
    pub async fn list_volumes(&self) -> Result<VolumesListResp> {
        let v: Value = self
            .get_json(&format!("{API_PREFIX}/volumes"), true, DEFAULT_TIMEOUT)
            .await?;
        let mut volumes = Vec::new();
        if let Some(arr) = v.get("Volumes").and_then(|a| a.as_array()) {
            for item in arr {
                volumes.push(parse_volume_summary(item));
            }
        }
        volumes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(VolumesListResp { volumes })
    }

    /// Delete one volume (`DELETE /containers/volumes/{name}`).
    pub async fn remove_volume(&self, name: &str, force: bool) -> Result<ContainerActionResp> {
        validate_id("volume name", name)?;
        let path = format!(
            "{API_PREFIX}/volumes/{}?force={force}",
            encode_query_component(name)
        );
        let resp = self
            .request(
                Method::DELETE,
                &path,
                None,
                false,
                DEFAULT_TIMEOUT,
                MAX_JSON_BODY,
            )
            .await?;
        check_status(&resp)?;
        Ok(ContainerActionResp {
            ok: true,
            id: name.to_string(),
            action: "remove_volume".into(),
            message: None,
        })
    }

    /// Prune unused volumes (`POST /containers/volumes/prune`).
    pub async fn prune_volumes(&self) -> Result<PruneResp> {
        let resp = self
            .request(
                Method::POST,
                &format!("{API_PREFIX}/volumes/prune"),
                None,
                false,
                DEFAULT_TIMEOUT,
                MAX_JSON_BODY,
            )
            .await?;
        check_status(&resp)?;
        let v: Value = serde_json::from_slice(&resp.body).unwrap_or(Value::Null);
        let deleted = string_array(&v, "VolumesDeleted");
        Ok(PruneResp {
            kind: "volumes".into(),
            deleted,
            space_reclaimed_bytes: u64_field(&v, "SpaceReclaimed"),
        })
    }

    /// List networks (`GET /containers/networks`).
    pub async fn list_networks(&self) -> Result<NetworksListResp> {
        let raw: Vec<Value> = self
            .get_json(&format!("{API_PREFIX}/networks"), true, DEFAULT_TIMEOUT)
            .await?;
        let mut networks: Vec<NetworkSummary> = raw.iter().map(parse_network_summary).collect();
        networks.sort_by(|a, b| {
            a.built_in
                .cmp(&b.built_in)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(NetworksListResp { networks })
    }

    /// Delete one network (`DELETE /containers/networks/{id}`).
    pub async fn remove_network(&self, id: &str) -> Result<ContainerActionResp> {
        validate_id("network id", id)?;
        let path = format!("{API_PREFIX}/networks/{}", encode_query_component(id));
        let resp = self
            .request(
                Method::DELETE,
                &path,
                None,
                false,
                DEFAULT_TIMEOUT,
                MAX_JSON_BODY,
            )
            .await?;
        check_status(&resp)?;
        Ok(ContainerActionResp {
            ok: true,
            id: id.to_string(),
            action: "remove_network".into(),
            message: None,
        })
    }

    /// Prune unused networks (`POST /containers/networks/prune`).
    pub async fn prune_networks(&self) -> Result<PruneResp> {
        let resp = self
            .request(
                Method::POST,
                &format!("{API_PREFIX}/networks/prune"),
                None,
                false,
                DEFAULT_TIMEOUT,
                MAX_JSON_BODY,
            )
            .await?;
        check_status(&resp)?;
        let v: Value = serde_json::from_slice(&resp.body).unwrap_or(Value::Null);
        Ok(PruneResp {
            kind: "networks".into(),
            deleted: string_array(&v, "NetworksDeleted"),
            space_reclaimed_bytes: 0,
        })
    }

    /// Prune stopped containers, unused networks, unused images, and optionally volumes.
    pub async fn system_prune(&self, req: &SystemPruneReq) -> Result<SystemPruneResp> {
        let c_resp = self
            .request(
                Method::POST,
                &format!("{API_PREFIX}/containers/prune"),
                None,
                false,
                DEFAULT_TIMEOUT,
                MAX_JSON_BODY,
            )
            .await?;
        check_status(&c_resp)?;
        let c_val: Value = serde_json::from_slice(&c_resp.body).unwrap_or(Value::Null);
        let containers_deleted = string_array(&c_val, "ContainersDeleted");
        let mut space_reclaimed = u64_field(&c_val, "SpaceReclaimed");

        let images = self.prune_images(!req.all_unused_images).await?;
        space_reclaimed = space_reclaimed.saturating_add(images.space_reclaimed_bytes);

        let networks = self.prune_networks().await?;

        let volumes_deleted = if req.include_volumes {
            let vols = self.prune_volumes().await?;
            space_reclaimed = space_reclaimed.saturating_add(vols.space_reclaimed_bytes);
            vols.deleted
        } else {
            Vec::new()
        };

        Ok(SystemPruneResp {
            containers_deleted,
            images_deleted: images.deleted,
            networks_deleted: networks.deleted,
            volumes_deleted,
            space_reclaimed_bytes: space_reclaimed,
        })
    }
}

fn split_image_ref(reference: &str) -> (&str, Option<&str>) {
    if reference.contains('@') {
        return (reference, None);
    }
    let last_slash = reference.rfind('/');
    if let Some(colon) = reference.rfind(':') {
        if last_slash.map(|sl| colon > sl).unwrap_or(true) {
            return (&reference[..colon], Some(&reference[colon + 1..]));
        }
    }
    (reference, None)
}
