//! One check of an agent pick — provider, model, effort — for every place
//! that stores or runs one (a workflow step on save, import and run, a
//! review's reviewers, a media production's planner): what the pickers offer
//! is what is accepted.

use otto_core::agent_runtime::RuntimeCapabilities;
use otto_core::api::ProviderModelOption;

use crate::provider_models::{probe_provider_models_cached, ModelProbe};
use crate::ProviderRegistry;

/// The answer: fine, not checkable now (the provider's model list is not
/// known — saving may go on, the run checks again), or refused with the
/// sentence to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickCheck {
    Ok,
    Unverified(String),
    Invalid(String),
}

/// Check a pick against the provider registry and the daemon's model
/// catalog. Empty model / effort mean the provider's default and always pass.
pub async fn check_pick(
    reg: &ProviderRegistry,
    provider: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> PickCheck {
    if let Err(e) = reg.ensure_enabled(provider) {
        return PickCheck::Invalid(e.to_string().trim_start_matches("invalid: ").to_string());
    }
    if !reg.is_enabled(provider) {
        return PickCheck::Invalid(format!("unknown provider '{provider}'"));
    }
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    let effort = effort.map(str::trim).filter(|e| !e.is_empty());
    if model.is_none() && effort.is_none() {
        return PickCheck::Ok;
    }
    let catalog = probe_provider_models_cached(&ModelProbe::for_registry(reg)).await;
    let levels = reg
        .runtime_provider_for(provider)
        .map(|p| RuntimeCapabilities::for_provider(p).efforts)
        .unwrap_or_default();
    check_against(
        catalog.get(provider).map(Vec::as_slice),
        &levels,
        provider,
        model,
        effort,
    )
}

/// [`check_pick`] over a known catalog. A model whose entry lists no
/// efforts takes the provider's levels (`levels`) instead of none.
pub fn check_against(
    models: Option<&[ProviderModelOption]>,
    levels: &[String],
    provider: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> PickCheck {
    let Some(models) = models.filter(|m| !m.is_empty()) else {
        return PickCheck::Unverified(format!(
            "{provider}'s model list is not available now — the pick is checked again when it runs"
        ));
    };
    let picked: Vec<&ProviderModelOption> = models
        .iter()
        .filter(|m| model.is_none_or(|id| m.id == id))
        .collect();
    if let (Some(model), true) = (model, picked.is_empty()) {
        return PickCheck::Invalid(format!(
            "{provider} has no model '{model}' — pick one it lists"
        ));
    }
    let takes = |m: &ProviderModelOption, e: &str| {
        let own = &m.supported_efforts;
        let list = if own.is_empty() {
            levels
        } else {
            own.as_slice()
        };
        list.iter().any(|s| s.eq_ignore_ascii_case(e))
    };
    match effort {
        Some(e) if !picked.iter().any(|m| takes(m, e)) => PickCheck::Invalid(format!(
            "{provider} {} has no effort '{e}' — pick one it lists",
            model.unwrap_or("models")
        )),
        _ => PickCheck::Ok,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, efforts: &[&str]) -> ProviderModelOption {
        ProviderModelOption {
            id: id.into(),
            label: id.into(),
            description: String::new(),
            default_effort: None,
            supported_efforts: efforts.iter().map(|e| e.to_string()).collect(),
        }
    }

    #[test]
    fn a_pick_is_ok_unverified_or_refused_with_the_pickers_words() {
        let levels: Vec<String> = ["low", "medium", "high"].map(String::from).to_vec();
        let models = [m("opus", &["low", "high", "max"]), m("sonnet", &[])];
        let check = |model, effort| check_against(Some(&models), &levels, "claude", model, effort);
        assert_eq!(check(Some("opus"), Some("max")), PickCheck::Ok);
        assert_eq!(
            check(None, Some("max")),
            PickCheck::Ok,
            "any model of the provider"
        );
        assert_eq!(
            check(Some("gpt-x"), None),
            PickCheck::Invalid("claude has no model 'gpt-x' — pick one it lists".into())
        );
        assert_eq!(
            check(Some("opus"), Some("minimal")),
            PickCheck::Invalid("claude opus has no effort 'minimal' — pick one it lists".into())
        );
        // DR-030: a model with no efforts of its own takes the provider's.
        assert_eq!(check(Some("sonnet"), Some("medium")), PickCheck::Ok);
        assert!(matches!(
            check(Some("sonnet"), Some("max")),
            PickCheck::Invalid(_)
        ));
        assert!(matches!(
            check_against(None, &levels, "codex", Some("gpt-5"), None),
            PickCheck::Unverified(_)
        ));
    }
}
