//! Account authorization. Bucket names are exact child identities; other AWS
//! operations require account-wide authority because their CLIs can fan out.
use axum::body::Body;
use futures_util::StreamExt;
use otto_core::access::{AccessActor, AccessPolicy, ResourceKind, ResourceRef};
use otto_core::domain::{Capability, Feature, User};
use otto_core::{Error, Id, Result};
use otto_rbac::resource_access::ResourceAccess;
use otto_state::{GrantsRepo, SqlitePool};

pub fn validate_policy(policy: &AccessPolicy) -> Result<()> {
    otto_core::access::validate_policy(policy)?;
    for rule in &policy.rules {
        if let Some(children) = &rule.children {
            for child in children {
                let name = child.strip_prefix("bucket:").ok_or_else(|| {
                    Error::Invalid("AWS child scopes must be bucket:<name>".into())
                })?;
                if name.is_empty()
                    || name.len() > 63
                    || !name.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.'
                    })
                {
                    return Err(Error::Invalid("invalid bucket scope".into()));
                }
            }
            if rule
                .operations
                .iter()
                .chain(&rule.grantable_operations)
                .any(|op| {
                    !matches!(
                        op.as_str(),
                        "discover" | "s3_list" | "s3_read" | "s3_write" | "s3_delete"
                    )
                })
            {
                return Err(Error::Invalid(
                    "only S3 object operations support bucket scopes".into(),
                ));
            }
        }
    }
    Ok(())
}

pub async fn allowed(
    pool: &SqlitePool,
    user: &User,
    id: &Id,
    operation: &str,
    bucket: Option<&str>,
) -> Result<bool> {
    if GrantsRepo::new(pool.clone())
        .capability_of(user, Feature::Aws)
        .await?
        < Capability::View
    {
        return Ok(false);
    }
    let resource = ResourceRef {
        kind: ResourceKind::AwsAccount,
        id: id.clone(),
        child: bucket.map(|b| format!("bucket:{b}")),
    };
    match ResourceAccess::new(pool.clone())
        .evaluate(user, &resource, operation)
        .await
    {
        Ok(decision) => Ok(decision.allowed),
        Err(Error::NotFound(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

pub async fn check(
    pool: &SqlitePool,
    user: &User,
    id: &Id,
    operation: &str,
    bucket: Option<&str>,
) -> Result<()> {
    if !allowed(pool, user, id, "discover", None).await? {
        return Err(Error::NotFound("AWS account".into()));
    }
    if allowed(pool, user, id, operation, bucket).await? {
        Ok(())
    } else {
        Err(Error::Forbidden(format!(
            "AWS account does not allow {operation}"
        )))
    }
}

/// Creator rules cannot exceed that creator's current feature authority.
pub async fn initialize(pool: &SqlitePool, user: &User, id: &Id) -> Result<()> {
    let grants = GrantsRepo::new(pool.clone());
    let mut operations = Vec::new();
    for op in otto_core::access::operations_for(ResourceKind::AwsAccount) {
        let (feature, needed) = match *op {
            "discover" | "metrics" => (Feature::Aws, Capability::View),
            "configure" | "manage_access" => (Feature::Aws, Capability::Admin),
            "s3_list" | "s3_read" => (Feature::AwsS3, Capability::View),
            "s3_buckets" => (Feature::AwsS3, Capability::Admin),
            "s3_write" | "s3_delete" => (Feature::AwsS3, Capability::Edit),
            "ec2_view" => (Feature::AwsEc2, Capability::View),
            "ec2_start" | "ec2_stop" | "ec2_reboot" | "ec2_terminate" => {
                (Feature::AwsEc2, Capability::Edit)
            }
            "sqs_view" | "sqs_receive" => (Feature::AwsSqs, Capability::View),
            "sqs_send" | "sqs_delete" | "sqs_purge" | "sqs_redrive" => {
                (Feature::AwsSqs, Capability::Edit)
            }
            "athena_view" => (Feature::AwsAthena, Capability::View),
            "athena_query" => (Feature::AwsAthena, Capability::Edit),
            "eks_view" => (Feature::AwsEks, Capability::View),
            "eks_import" => (Feature::AwsEks, Capability::Edit),
            "rds_view" => (Feature::AwsRds, Capability::View),
            _ => continue,
        };
        if grants.capability_of(user, feature).await? >= needed {
            operations.push((*op).to_string());
        }
    }
    let actor = AccessActor {
        real_user_id: user.id.clone(),
        effective_user_id: None,
    };
    otto_state::resource_access::ResourceAccessRepo::new(pool.clone())
        .initialize_owner_policy(
            ResourceKind::AwsAccount,
            id,
            &user.id,
            &operations,
            &operations,
            &actor,
        )
        .await?;
    Ok(())
}

/// Stop a download as soon as its grant is revoked, including an idle stream.
/// Dropping the inner body also drops the AWS child process guard.
pub fn guard_body(
    body: Body,
    pool: SqlitePool,
    user: User,
    id: Id,
    bucket: Option<String>,
    operation: &'static str,
) -> Body {
    let stream = futures_util::stream::unfold(
        (body.into_data_stream(), pool, user, id, bucket),
        move |(mut stream, pool, user, id, bucket)| async move {
            loop {
                let current = match otto_state::UsersRepo::new(pool.clone()).get(&user.id).await {
                    Ok(user) => user,
                    Err(_) => return None,
                };
                if !matches!(GrantsRepo::new(pool.clone()).capability_of(&current, Feature::AwsS3).await, Ok(cap) if cap >= Capability::View)
                {
                    return None;
                }
                if !matches!(
                    allowed(&pool, &current, &id, operation, bucket.as_deref()).await,
                    Ok(true)
                ) {
                    return None;
                }
                tokio::select! {
                    data = stream.next() => return data.map(|data| (data, (stream, pool, user, id, bucket))),
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
                }
            }
        },
    );
    Body::from_stream(stream)
}

/// Native credential attachment is host authority, not delegated resource configuration.
pub fn require_setup_authority(user: &User) -> Result<()> {
    if user.is_root && !user.disabled {
        Ok(())
    } else {
        Err(Error::Forbidden(
            "only root can attach or change native cloud credentials".into(),
        ))
    }
}

/// Legacy resources still require the original Admin feature tier to expose configuration.
pub async fn can_configure(pool: &SqlitePool, user: &User, id: &Id) -> Result<bool> {
    if !allowed(pool, user, id, "configure", None).await? {
        return Ok(false);
    }
    let policy = otto_state::resource_access::ResourceAccessRepo::new(pool.clone())
        .get_policy(ResourceKind::AwsAccount, id)
        .await?;
    Ok(policy.mode != otto_core::access::AccessMode::Legacy
        || GrantsRepo::new(pool.clone())
            .capability_of(user, Feature::Aws)
            .await?
            >= Capability::Admin)
}
