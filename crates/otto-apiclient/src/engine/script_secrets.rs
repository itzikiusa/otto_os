//! What a saved request's scripts see of the environment's Keychain secrets:
//! never their values. Each secret variable holds its own `{{KEY}}`
//! placeholder instead, which the engine substitutes only when it builds the
//! request — after the script ran and after the secret ↔ host binding decided
//! where every placeholder may go. A script that moves a placeholder to
//! another host is caught by that binding check; one that computes with it
//! (HMAC signing, base64 of `user:secret`) would send garbage, so it is
//! refused with a message saying why.

use super::*;

/// The environment secrets still holding their Keychain value in a run's
/// variables (a runtime override that replaced one carries no secret).
#[derive(Debug, Default)]
pub struct ScriptSecrets {
    values: BTreeMap<String, String>,
}

impl ScriptSecrets {
    /// The secrets among `secret_keys` whose value in `vars` is still the
    /// Keychain value from `env_blob`.
    pub fn new(
        vars: &serde_json::Map<String, Value>,
        env_blob: &BTreeMap<String, String>,
        secret_keys: &[String],
    ) -> Self {
        let values = secret_keys
            .iter()
            .filter_map(|key| {
                let secret = env_blob.get(key)?;
                (vars.get(key).and_then(Value::as_str) == Some(secret.as_str()))
                    .then(|| (key.clone(), secret.clone()))
            })
            .collect();
        Self { values }
    }

    /// The text a script reads in place of secret `key`.
    pub fn placeholder(key: &str) -> String {
        format!("{{{{{key}}}}}")
    }

    /// The script's view of `vars`: every secret replaced by its placeholder.
    pub fn script_vars(&self, vars: &serde_json::Map<String, Value>) -> BTreeMap<String, String> {
        let mut out = string_vars(vars);
        for key in self.values.keys() {
            out.insert(key.clone(), Self::placeholder(key));
        }
        out
    }

    /// Put each secret's value back where the script left its placeholder
    /// untouched, so merging the script's variables never stores a
    /// placeholder over the secret. A value the script SET is the script's.
    pub fn restore(&self, script_vars: &mut BTreeMap<String, String>) {
        for (key, value) in &self.values {
            if script_vars.get(key) == Some(&Self::placeholder(key)) {
                script_vars.insert(key.clone(), value.clone());
            }
        }
    }

    /// Refuse a pre-request script that names a secret and changed the
    /// request, yet left that secret's placeholder nowhere intact (not in the
    /// request, not in a variable): it computed with the placeholder, and the
    /// request it built would carry a wrong value.
    pub fn check_pre(
        &self,
        code: &str,
        before: &ScriptRequest,
        out: &api_scripts::ScriptOutcome,
    ) -> Result<(), String> {
        let Some(after) = &out.request else {
            return Ok(());
        };
        let changed = after.method != before.method
            || after.url != before.url
            || after.headers != before.headers
            || after.body != before.body;
        if !changed {
            return Ok(());
        }
        let sent = serde_json::to_string(&json!([after.url, after.headers, after.body]))
            .unwrap_or_default();
        for key in self.values.keys() {
            let placeholder = Self::placeholder(key);
            if !names_key(code, key)
                || sent.contains(&placeholder)
                || out
                    .vars
                    .iter()
                    .any(|(name, value)| name != key && value.contains(&placeholder))
            {
                continue;
            }
            return Err(format!(
                "the pre-request script computes with secret '{key}', but scripts only see the placeholder {placeholder}, never the value — the request would carry a wrong credential. Put {placeholder} in the request itself (signing with a stored secret is not supported in saved or automated runs)"
            ));
        }
        Ok(())
    }
}

/// Whether `code` names `key` as a string literal (`'KEY'`, `"KEY"`, `` `KEY` ``).
fn names_key(code: &str, key: &str) -> bool {
    ['\'', '"', '`']
        .iter()
        .any(|quote| code.contains(&format!("{quote}{key}{quote}")))
}
