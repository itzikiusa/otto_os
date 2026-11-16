use super::*;

static TEST_CACHE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// `/meta` answers from a stale catalog at once; one background refresh
/// replaces it (no provider here, so the refresh is an empty map).
#[tokio::test]
async fn stale_catalog_is_served_then_refreshed_in_the_background() {
    let _serial = TEST_CACHE_LOCK.lock().await;
    use std::sync::atomic::Ordering;
    let marker = HashMap::from([("stale".to_string(), Vec::new())]);
    let aged = Instant::now()
        .checked_sub(Duration::from_secs(600))
        .unwrap();
    let epoch = CACHE_GEN.load(Ordering::Acquire);
    *CACHE.lock().await = Some((epoch, aged, false, ModelProbe::default(), marker));
    let served = probe_provider_models_cached(&ModelProbe::default()).await;
    assert!(served.contains_key("stale"), "served stale");
    for _ in 0..200 {
        if !REFRESHING.load(Ordering::Acquire) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(probe_provider_models_cached(&ModelProbe::default())
        .await
        .is_empty());
}

#[tokio::test]
async fn invalidate_provider_models_cache_forces_immediate_rediscovery() {
    let _serial = TEST_CACHE_LOCK.lock().await;
    use std::sync::atomic::Ordering;
    let epoch = CACHE_GEN.load(Ordering::Acquire);
    let marker = HashMap::from([("cached-only".to_string(), Vec::new())]);
    *CACHE.lock().await = Some((epoch, Instant::now(), false, ModelProbe::default(), marker));
    assert!(probe_provider_models_cached(&ModelProbe::default())
        .await
        .contains_key("cached-only"));

    invalidate_caches_after_login(Some("claude"));
    assert!(
        probe_provider_models_cached(&ModelProbe::default())
            .await
            .is_empty(),
        "invalidated cache must not serve pre-refresh snapshot"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn cold_discovery_returns_within_cold_budget_then_populates_slow_cli() {
    let _serial = TEST_CACHE_LOCK.lock().await;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::Ordering;

    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("slow-opencode");
    std::fs::write(
        &script,
        "#!/bin/sh\nsleep 0.45\nprintf 'openai/gpt-slow-test\\n'\n",
    )
    .unwrap();
    let mut perms = std::fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script, perms).unwrap();

    *CACHE.lock().await = None;
    let probe = ModelProbe {
        families: vec![
            ("claude".into(), "claude".into()),
            ("opencode".into(), "opencode".into()),
        ],
        opencode_program: Some(script.to_string_lossy().into_owned()),
        ..Default::default()
    };

    let t0 = Instant::now();
    let (provisional, is_prov) = probe_provider_models_cached_with_status(&probe).await;
    let elapsed = t0.elapsed();
    assert!(is_prov, "cold timeout snapshot must report provisional");
    assert!(
        elapsed < Duration::from_millis(420),
        "cold /meta exceeded inline budget: {elapsed:?}"
    );
    assert!(
        provisional.get("claude").is_some_and(|m| !m.is_empty()),
        "provisional catalog serves local/fallback claude models immediately"
    );
    assert!(
        !provisional.contains_key("opencode"),
        "slow CLI has not finished yet"
    );

    for _ in 0..100 {
        if !REFRESHING.load(Ordering::Acquire) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (refreshed, is_prov_after) = probe_provider_models_cached_with_status(&probe).await;
    assert!(!is_prov_after, "completed CLI discovery clears provisional");
    assert_eq!(
        refreshed
            .get("opencode")
            .map(|v| v.iter().map(|m| m.id.as_str()).collect::<Vec<_>>()),
        Some(vec!["openai/gpt-slow-test"])
    );
}

#[cfg(unix)]
#[tokio::test]
async fn unauthenticated_cli_stays_provisional_and_refreshes_after_login() {
    let _serial = TEST_CACHE_LOCK.lock().await;
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let state_file = dir.path().join("authed");
    let agy_bin = dir.path().join("fake-agy");
    let pi_bin = dir.path().join("fake-pi");
    let grok_bin = dir.path().join("fake-grok");
    std::fs::write(
        &agy_bin,
        format!(
            "#!/bin/sh\nif [ -f '{}' ]; then\n  printf 'gemini-4-live-low\\tGemini 4 Live (Low)\\ngemini-4-live-high\\tGemini 4 Live (High)\\n'\nelse\n  exit 1\nfi\n",
            state_file.display()
        ),
    )
    .unwrap();
    std::fs::write(
        &pi_bin,
        format!(
            "#!/bin/sh\nif [ -f '{}' ]; then\n  printf 'provider model context max-out thinking images\\nopenai gpt-live 200K 32K yes yes\\n'\nfi\n",
            state_file.display()
        ),
    )
    .unwrap();
    std::fs::write(
        &grok_bin,
        format!(
            "#!/bin/sh\nif [ -f '{}' ]; then\n  printf 'Available models:\\n  * grok-live (default)\\n'\nfi\n",
            state_file.display()
        ),
    )
    .unwrap();
    for b in [&agy_bin, &pi_bin, &grok_bin] {
        let mut perms = std::fs::metadata(b).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(b, perms).unwrap();
    }

    *CACHE.lock().await = None;
    let probe = ModelProbe {
        families: vec![
            ("agy".into(), "agy".into()),
            ("pi".into(), "pi".into()),
            ("grok".into(), "grok".into()),
        ],
        agy_program: Some(agy_bin.to_string_lossy().into_owned()),
        pi_program: Some(pi_bin.to_string_lossy().into_owned()),
        grok_program: Some(grok_bin.to_string_lossy().into_owned()),
        ..Default::default()
    };

    let (before, is_prov) = probe_provider_models_cached_with_status(&probe).await;
    assert!(
        is_prov,
        "fallback agy + empty pi/grok before login must remain provisional"
    );
    assert!(!before.contains_key("pi"));
    assert!(!before.contains_key("grok"));

    // Simulate login completing and calling the post-login invalidation hook.
    std::fs::write(&state_file, "ok").unwrap();
    invalidate_caches_after_login(None);

    let (after, is_prov_after) = probe_provider_models_cached_with_status(&probe).await;
    assert!(!is_prov_after);
    assert_eq!(after["agy"][0].id, "gemini-4-live");
    assert_eq!(after["pi"][0].id, "openai/gpt-live");
    assert_eq!(after["grok"][0].id, "grok-live");
}

#[test]
fn agy_parser_strips_ansi_csi_spinners_and_colors() {
    let raw = "\x1b[?25l\x1b[2K\r\x1b[36m⠋ Fetching models...\x1b[0m\r\x1b[2K\
               \x1b[32mgemini-4-carbon-low\x1b[0m\tGemini 4 Carbon (Low)\n\
               \x1b[32mgemini-4-carbon-high\x1b[0m\tGemini 4 Carbon (High)\n\
               \x1b[?25h";
    let got = parse_agy_models_output(raw);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, "gemini-4-carbon");
    assert_eq!(got[0].label, "Gemini 4 Carbon");
    assert_eq!(got[0].supported_efforts, vec!["low", "high"]);
}

#[test]
fn parses_claude_codex_and_agy_catalogs() {
    let claude = parse_claude_catalog_json(
        r#"{"catalog":{"config":{"models":[
            {"id":"claude-old","name":"Old","section":"overflow","runtime":{"default_effort":"high","effort_levels":["low","high"]}},
            {"id":"claude-opus-5-5","name":"Opus 5.5","description":"Everyday tasks","section":"main","runtime":{"default_effort":"medium","effort_levels":["low","medium","high","xhigh","max"]}}
        ]}}}"#,
    );
    assert_eq!(claude.len(), 2);
    assert_eq!(
        claude[0].id, "claude-opus-5-5",
        "main section sorts ahead of overflow"
    );
    assert_eq!(claude[0].default_effort.as_deref(), Some("medium"));
    assert_eq!(claude[0].supported_efforts.len(), 5);

    let codex = parse_codex_models_cache_json(
        r#"{"models":[
            {"slug":"gpt-hidden","display_name":"Hidden","visibility":"hide"},
            {"slug":"gpt-6.1-sol","display_name":"GPT-6.1-Sol","description":"Workhorse","visibility":"list","default_reasoning_level":"low","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"},{"effort":"max"}]}
        ]}"#,
    );
    assert_eq!(codex.len(), 1);
    assert_eq!(codex[0].id, "gpt-6.1-sol");
    assert_eq!(codex[0].supported_efforts, vec!["low", "high", "max"]);

    let agy = parse_agy_models_output(
        "\r⠋ Fetching available models...\rgemini-4-carbon-medium         Gemini 4 Carbon (Medium)\n\
         gemini-4-carbon-high           Gemini 4 Carbon (High)\n\
         gemini-4-carbon-low            Gemini 4 Carbon (Low)\n\
         gemini-4-hydrogen-high         Gemini 4 Hydrogen (High)\n\
         gemini-4-hydrogen-xhigh        Gemini 4 Hydrogen (XHigh)\n\
         gemini-pro-orchestrator        Orchestrator (Carbon)\n\
         claude-opus-5-5-low            Claude Opus 5.5 (Low)\n\
         claude-opus-5-5-medium         Claude Opus 5.5 (Medium)\n\
         claude-opus-5-5-high           Claude Opus 5.5 (High)\n\
         gpt-oss-120b-medium            GPT-OSS 120B (Medium)\n",
    );
    assert_eq!(
        agy.iter()
            .map(|m| (m.id.as_str(), m.label.as_str()))
            .collect::<Vec<_>>(),
        [
            ("gemini-4-carbon", "Gemini 4 Carbon"),
            ("gemini-4-hydrogen", "Gemini 4 Hydrogen"),
            ("gemini-pro-orchestrator", "Orchestrator (Carbon)"),
            ("claude-opus-5-5", "Claude Opus 5.5"),
            ("gpt-oss-120b-medium", "GPT-OSS 120B"),
        ]
    );
    assert_eq!(agy[0].default_effort.as_deref(), Some("medium"));
    assert_eq!(agy[0].supported_efforts, vec!["low", "medium", "high"]);
    assert_eq!(agy[1].default_effort.as_deref(), Some("high"));
    assert_eq!(agy[1].supported_efforts, vec!["high", "xhigh"]);
    assert_eq!(agy[2].default_effort, None);
    assert!(agy[2].supported_efforts.is_empty());
    assert_eq!(agy[3].default_effort.as_deref(), Some("medium"));
    assert_eq!(agy[3].supported_efforts, vec!["low", "medium", "high"]);
    assert_eq!(agy[4].default_effort.as_deref(), Some("medium"));
    assert_eq!(agy[4].supported_efforts, vec!["medium"]);

    let grok = parse_grok_models_output(
        "You are not authenticated.\n\nDefault model: grok-4.6\n\nAvailable models:\n  * grok-4.6 (default)\n  - grok-4.5\n\nAvailable models:\n  * grok-4.6 (default)\n",
    );
    assert_eq!(
        grok.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["grok-4.6", "grok-4.5"]
    );

    let pi = parse_pi_models_output(
        "provider      model                context  max-out  thinking  images\nopenai-codex  gpt-5.5              272K     128K     yes       yes   \nx  tiny  8K  1K  no  no\n",
    );
    assert_eq!(pi[0].id, "openai-codex/gpt-5.5");
    assert_eq!(pi[0].supported_efforts.len(), 4);
    assert!(pi[1].supported_efforts.is_empty());

    let oc =
        parse_opencode_models_output("anthropic/claude-sonnet-4\nopenai/gpt-5\nsome log line\n");
    assert_eq!(oc.len(), 2);
}

#[test]
fn parses_real_pi_1_5_and_opencode_1_19_output() {
    let pi = parse_pi_models_output(
        "provider      model          context  max-out  thinking  images\n\
         openai-codex  gpt-5.6-luna   272K     128K     yes       yes   \n\
         openai-codex  gpt-6.1-terra  500K     128K     yes       yes   \n\
         opencode      big-pickle     200K     32K      yes       no    \n\
         \n",
    );
    assert_eq!(
        pi.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        [
            "openai-codex/gpt-5.6-luna",
            "openai-codex/gpt-6.1-terra",
            "opencode/big-pickle"
        ]
    );
    assert_eq!(pi[1].label, "gpt-6.1-terra");
    assert_eq!(pi[1].description, "openai-codex · 500K context");
    assert_eq!(pi[0].default_effort.as_deref(), Some("medium"));

    let oc = parse_opencode_models_output(
        "opencode/beam-free\nopencode/big-pickle\ngithub-copilot/claude-haiku-4.5\n\
         google/gemini-3.8-flash\nopenai/gpt-5.6-luna-fast\n\
         \n[auto-update-checker] Package not found, nothing to invalidate: x\n",
    );
    assert_eq!(
        oc.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        [
            "opencode/beam-free",
            "opencode/big-pickle",
            "github-copilot/claude-haiku-4.5",
            "google/gemini-3.8-flash",
            "openai/gpt-5.6-luna-fast"
        ]
    );
}

#[tokio::test]
async fn custom_providers_share_their_family_catalog() {
    let probe = ModelProbe {
        families: vec![
            ("claude".into(), "claude".into()),
            ("claude-work".into(), "claude".into()),
        ],
        ..Default::default()
    };
    let got = discover_models(&probe).await;
    assert!(!got["claude-work"].is_empty());
    assert_eq!(got["claude-work"], got["claude"]);
}
