//! The environment every `aws` subprocess gets — directly (otto-aws) or as
//! kubectl's `aws eks get-token` credential plugin (otto-k8s, EKS clusters).
//!
//! One builder for both crates: otto-k8s used to keep a "minimal mirror" that
//! drifted — it blanked `AWS_PROFILE` (which CLI v2 reads as a profile named
//! `""` and fails every call on) and ignored the account's custom endpoint.

/// Static credentials as the CLI wants them in the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwsStaticCreds {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

/// How an account authenticates.
#[derive(Debug, Clone, Copy)]
pub enum AwsEnvAuth<'a> {
    /// A named profile from `~/.aws/config` (`None` / blank ⇒ the CLI default).
    Profile(Option<&'a str>),
    /// Access keys (`None` ⇒ none available; the CLI reports it).
    AccessKeys(Option<&'a AwsStaticCreds>),
}

/// Variables a spawn must REMOVE from the inherited daemon environment before
/// applying [`build_aws_env`]'s output (`Command::env_remove`). A stray
/// `AWS_PROFILE` must not override access keys — and it cannot be blanked
/// instead: CLI v2 treats `AWS_PROFILE=""` as a profile literally named `""`
/// and fails every call with `The config profile () could not be found`.
/// Profile mode puts `AWS_PROFILE` back through the env itself.
pub const AWS_ENV_REMOVE: &[&str] = &["AWS_PROFILE"];

/// Apply a spawn env that carries [`build_aws_env`]'s output: first
/// `env_remove` every [`AWS_ENV_REMOVE`] name inherited from the daemon, then
/// set `env`. Returns `cmd` so it sits inside a builder chain. Every `aws` /
/// `kubectl` (EKS `aws eks get-token`) spawn goes through this, so the rule
/// lives in one place.
pub fn apply_aws_env<I, K, V>(
    cmd: &mut tokio::process::Command,
    env: I,
) -> &mut tokio::process::Command
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<std::ffi::OsStr>,
    V: AsRef<std::ffi::OsStr>,
{
    for k in AWS_ENV_REMOVE {
        cmd.env_remove(k);
    }
    cmd.envs(env)
}

/// The `aws` subprocess env (contract §1 "Auth injection"). Profile mode sets
/// `AWS_PROFILE`; keys mode sets the three `AWS_*` credential vars. Both set
/// the region (+ `AWS_DEFAULT_REGION` for older code paths), disable the pager
/// and the v2 auto-prompt. A custom `endpoint_url` (LocalStack, VPC endpoints,
/// S3-compatible stores) becomes `AWS_ENDPOINT_URL` — honoured by every CLI
/// v2 ≥ 2.13 command, `s3 cp` included — plus `AWS_EC2_METADATA_DISABLED=true`
/// so the CLI never stalls on the IMDS credential provider when pointed at a
/// non-AWS host. Never emits a blank `AWS_PROFILE` (see [`AWS_ENV_REMOVE`]).
pub fn build_aws_env(
    auth: AwsEnvAuth<'_>,
    region: &str,
    endpoint_url: Option<&str>,
) -> Vec<(String, String)> {
    let mut env = vec![
        ("AWS_REGION".to_string(), region.to_string()),
        ("AWS_DEFAULT_REGION".to_string(), region.to_string()),
        ("AWS_PAGER".to_string(), String::new()),
        ("AWS_CLI_AUTO_PROMPT".to_string(), "off".to_string()),
    ];
    if let Some(u) = endpoint_url.map(str::trim).filter(|u| !u.is_empty()) {
        env.push(("AWS_ENDPOINT_URL".into(), u.to_string()));
        env.push(("AWS_EC2_METADATA_DISABLED".into(), "true".into()));
    }
    match auth {
        AwsEnvAuth::Profile(profile) => {
            if let Some(p) = profile.map(str::trim).filter(|p| !p.is_empty()) {
                env.push(("AWS_PROFILE".into(), p.to_string()));
            }
        }
        AwsEnvAuth::AccessKeys(creds) => {
            if let Some(c) = creds {
                env.push(("AWS_ACCESS_KEY_ID".into(), c.access_key_id.clone()));
                env.push(("AWS_SECRET_ACCESS_KEY".into(), c.secret_access_key.clone()));
                if let Some(t) = c.session_token.as_deref().filter(|t| !t.is_empty()) {
                    env.push(("AWS_SESSION_TOKEN".into(), t.to_string()));
                }
            }
        }
    }
    env
}

#[cfg(test)]
mod tests {
    /// The inherited `AWS_PROFILE` is removed, and an env that sets it (profile
    /// mode) puts it back.
    #[test]
    fn apply_aws_env_removes_the_inherited_profile_first() {
        let mut cmd = tokio::process::Command::new("true");
        cmd.env("AWS_PROFILE", "daemon-default");
        super::apply_aws_env(&mut cmd, [("AWS_REGION", "eu-west-1")]);
        let envs: Vec<_> = cmd.as_std().get_envs().collect();
        assert!(envs.contains(&(std::ffi::OsStr::new("AWS_PROFILE"), None)));
        assert!(envs.contains(&(
            std::ffi::OsStr::new("AWS_REGION"),
            Some(std::ffi::OsStr::new("eu-west-1"))
        )));
        let mut cmd = tokio::process::Command::new("true");
        super::apply_aws_env(&mut cmd, [("AWS_PROFILE", "dev")]);
        let envs: Vec<_> = cmd.as_std().get_envs().collect();
        assert!(envs.contains(&(
            std::ffi::OsStr::new("AWS_PROFILE"),
            Some(std::ffi::OsStr::new("dev"))
        )));
    }

    use super::*;

    fn get<'a>(env: &'a [(String, String)], k: &str) -> Option<&'a str> {
        env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }

    #[test]
    fn profile_mode_sets_the_profile_and_common_vars() {
        let env = build_aws_env(AwsEnvAuth::Profile(Some("dev-sso")), "eu-west-1", None);
        assert_eq!(get(&env, "AWS_PROFILE"), Some("dev-sso"));
        assert_eq!(get(&env, "AWS_REGION"), Some("eu-west-1"));
        assert_eq!(get(&env, "AWS_DEFAULT_REGION"), Some("eu-west-1"));
        assert_eq!(get(&env, "AWS_PAGER"), Some(""));
        assert_eq!(get(&env, "AWS_CLI_AUTO_PROMPT"), Some("off"));
        assert_eq!(get(&env, "AWS_ACCESS_KEY_ID"), None);
        assert_eq!(get(&env, "AWS_ENDPOINT_URL"), None);
    }

    #[test]
    fn a_blank_profile_is_omitted_not_blanked() {
        let env = build_aws_env(AwsEnvAuth::Profile(Some("  ")), "us-east-1", None);
        assert_eq!(get(&env, "AWS_PROFILE"), None);
    }

    #[test]
    fn key_mode_sets_the_keys_and_never_a_blank_profile() {
        let creds = AwsStaticCreds {
            access_key_id: "AKIAEXAMPLE".into(),
            secret_access_key: "s3cr3t".into(),
            session_token: Some("tok".into()),
        };
        let env = build_aws_env(
            AwsEnvAuth::AccessKeys(Some(&creds)),
            "us-east-1",
            Some(" http://localhost:4566 "),
        );
        assert_eq!(get(&env, "AWS_ACCESS_KEY_ID"), Some("AKIAEXAMPLE"));
        assert_eq!(get(&env, "AWS_SECRET_ACCESS_KEY"), Some("s3cr3t"));
        assert_eq!(get(&env, "AWS_SESSION_TOKEN"), Some("tok"));
        assert_eq!(
            get(&env, "AWS_PROFILE"),
            None,
            "removed by the spawn, never blanked"
        );
        assert_eq!(get(&env, "AWS_ENDPOINT_URL"), Some("http://localhost:4566"));
        assert_eq!(get(&env, "AWS_EC2_METADATA_DISABLED"), Some("true"));
        assert_eq!(AWS_ENV_REMOVE, &["AWS_PROFILE"]);
    }
}
