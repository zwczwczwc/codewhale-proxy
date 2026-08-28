use crate::config::ProviderConfig;
use serde_json::Value;

/// Canonical effort tiers, ordered low → high.
///
/// `xhigh` (Claude Code's interactive-mode effort intent) and `max` (the
/// common upstream API spelling) are the SAME tier: they are two names for
/// "top". Tier identity — not a fixed alias table — is what lets an inbound
/// level fall back to the nearest supported upstream level when the exact
/// name is absent from a provider's `effort_map`.
pub fn effort_tier(effort: &str) -> Option<u8> {
    match effort {
        "off" | "disabled" | "none" | "false" => Some(0),
        "minimal" => Some(1),
        "low" => Some(2),
        "medium" => Some(3),
        "high" => Some(4),
        "xhigh" | "max" => Some(5),
        _ => None,
    }
}

/// Resolve the wire effort for an inbound intent against one provider.
///
/// Resolution order:
/// 1. exact `effort_map` hit — byte-for-byte compatible with every existing
///    config (deepseek xhigh→max, kimi medium→low, glm xhigh→max, ...);
/// 2. same-tier member of the provider's SUPPORTED set (`effort_map.values()`,
///    derived — no new config surface): inbound `max` passes through to glm
///    which supports `max`, even though its map has no `max` key;
/// 3. nearest supported tier by minimal rank distance, ties resolved DOWNWARD
///    (cheaper than over-shooting);
/// 4. nothing supported at all → lowest-ranked supported value.
pub fn resolve_effort(inbound: &str, provider: &ProviderConfig) -> String {
    if let Some(mapped) = provider.effort_map.get(inbound) {
        return mapped.clone();
    }
    let inbound_tier = match effort_tier(inbound) {
        Some(t) => t,
        None => return "high".to_string(), // legacy unknown-level fallback
    };
    // Supported set: distinct upstream values with their tiers.
    let mut supported: Vec<(u8, &str)> = provider
        .effort_map
        .values()
        .map(|v| (effort_tier(v).unwrap_or(u8::MAX), v.as_str()))
        .collect();
    supported.sort();
    supported.dedup();
    if supported.is_empty() {
        return "high".to_string(); // degenerate config fallback
    }
    // Same-tier member first (e.g. inbound max ↔ upstream supports max).
    if let Some(&(_, val)) = supported.iter().find(|(tier, _)| *tier == inbound_tier) {
        return val.to_string();
    }
    // Nearest tier; ties go to the LOWER tier (never overshoot).
    let best = supported
        .iter()
        .min_by_key(|&&(tier, _)| (tier.abs_diff(inbound_tier), tier))
        .expect("supported is non-empty");
    best.1.to_string()
}

/// Applies reasoning_effort to the OpenAI request body.
/// Now config-driven: uses ProviderConfig (effort_map, thinking_param, disable_thinking)
/// instead of hardcoded provider match branches.
#[allow(dead_code)]
pub fn apply_reasoning_effort(body: &mut Value, effort: Option<&str>, provider: &ProviderConfig) {
    let effort = match effort {
        Some(e) => e.to_lowercase(),
        None => return,
    };

    let effort = effort.trim();

    match effort {
        "off" | "disabled" | "none" | "false" => {
            if provider.disable_thinking {
                // Cannot turn off thinking; set to lowest effort
                let lowest = provider
                    .effort_map
                    .get("low")
                    .cloned()
                    .unwrap_or_else(|| "low".to_string());
                body[&provider.effort_param] = serde_json::json!(lowest);
                if let Some(obj) = body.as_object_mut() {
                    if let Some(ref tp) = provider.thinking_param {
                        obj.remove(tp);
                    }
                }
            } else {
                // Set thinking.type = disabled
                if let Some(ref tp) = provider.thinking_param {
                    body[tp] = serde_json::json!({
                        "type": provider.thinking_type_disabled.as_deref().unwrap_or("disabled")
                    });
                }
                // Remove reasoning_effort
                if let Some(obj) = body.as_object_mut() {
                    obj.remove(&provider.effort_param);
                }
            }
        }
        _ => {
            // Map effort through effort_map, default to "high" if unknown
            let mapped = provider
                .effort_map
                .get(effort)
                .cloned()
                .unwrap_or_else(|| "high".to_string());
            body[&provider.effort_param] = serde_json::json!(mapped);

            // Set thinking.type = enabled if provider supports it
            if let Some(ref tp) = provider.thinking_param {
                body[tp] = serde_json::json!({
                    "type": provider.thinking_type_enabled.as_deref().unwrap_or("enabled")
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn deepseek_provider() -> ProviderConfig {
        ProviderConfig {
            reasoning_field: "reasoning_content".to_string(),
            reasoning_field_alt: vec![],
            thinking_param: Some("thinking".to_string()),
            thinking_type_enabled: Some("enabled".to_string()),
            thinking_type_disabled: Some("disabled".to_string()),
            disable_thinking: false,
            effort_param: "reasoning_effort".to_string(),
            effort_map: {
                let mut m = HashMap::new();
                m.insert("low".to_string(), "high".to_string());
                m.insert("medium".to_string(), "high".to_string());
                m.insert("high".to_string(), "high".to_string());
                m.insert("max".to_string(), "max".to_string());
                m.insert("xhigh".to_string(), "max".to_string());
                m
            },
            responses_reasoning_summary: None,
            cache_policy: None,
        }
    }

    fn kimi_provider() -> ProviderConfig {
        ProviderConfig {
            reasoning_field: "reasoning".to_string(),
            reasoning_field_alt: vec![],
            thinking_param: None,
            thinking_type_enabled: None,
            thinking_type_disabled: None,
            disable_thinking: true,
            effort_param: "reasoning_effort".to_string(),
            effort_map: {
                let mut m = HashMap::new();
                m.insert("low".to_string(), "low".to_string());
                m.insert("medium".to_string(), "medium".to_string());
                m.insert("high".to_string(), "high".to_string());
                m.insert("max".to_string(), "max".to_string());
                m.insert("xhigh".to_string(), "max".to_string());
                m
            },
            responses_reasoning_summary: None,
            cache_policy: None,
        }
    }

    #[test]
    fn test_deepseek_max() {
        let mut body = serde_json::json!({});
        apply_reasoning_effort(&mut body, Some("max"), &deepseek_provider());
        assert_eq!(body["reasoning_effort"], "max");
        assert_eq!(body["thinking"]["type"], "enabled");
    }

    #[test]
    fn test_deepseek_off() {
        let mut body = serde_json::json!({});
        apply_reasoning_effort(&mut body, Some("off"), &deepseek_provider());
        assert_eq!(body["thinking"]["type"], "disabled");
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_deepseek_low() {
        let mut body = serde_json::json!({});
        apply_reasoning_effort(&mut body, Some("low"), &deepseek_provider());
        assert_eq!(body["reasoning_effort"], "high");
        assert_eq!(body["thinking"]["type"], "enabled");
    }

    #[test]
    fn test_no_effort() {
        let mut body = serde_json::json!({});
        apply_reasoning_effort(&mut body, None, &deepseek_provider());
        assert!(body.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_kimi_max() {
        let mut body = serde_json::json!({});
        apply_reasoning_effort(&mut body, Some("max"), &kimi_provider());
        assert_eq!(body["reasoning_effort"], "max");
        // Kimi does NOT support thinking.type
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn test_kimi_xhigh() {
        let mut body = serde_json::json!({});
        apply_reasoning_effort(&mut body, Some("xhigh"), &kimi_provider());
        assert_eq!(body["reasoning_effort"], "max");
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn test_kimi_off() {
        let mut body = serde_json::json!({});
        apply_reasoning_effort(&mut body, Some("off"), &kimi_provider());
        // Kimi can't turn off thinking, falls back to lowest effort
        assert_eq!(body["reasoning_effort"], "low");
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn test_kimi_low() {
        let mut body = serde_json::json!({});
        apply_reasoning_effort(&mut body, Some("low"), &kimi_provider());
        assert_eq!(body["reasoning_effort"], "low");
        assert!(body.get("thinking").is_none());
    }
}

#[cfg(test)]
mod resolve_effort_tests {
    use super::*;
    use std::collections::HashMap;

    fn provider(map: &[(&str, &str)]) -> ProviderConfig {
        let mut m = HashMap::new();
        for (k, v) in map {
            m.insert(k.to_string(), v.to_string());
        }
        ProviderConfig {
            reasoning_field: "reasoning_content".to_string(),
            reasoning_field_alt: vec![],
            thinking_param: Some("thinking".to_string()),
            thinking_type_enabled: Some("enabled".to_string()),
            thinking_type_disabled: Some("disabled".to_string()),
            disable_thinking: false,
            effort_param: "reasoning_effort".to_string(),
            effort_map: m,
            responses_reasoning_summary: None,
            cache_policy: None,
        }
    }

    #[test]
    fn tier_identity_groups_xhigh_and_max() {
        assert_eq!(effort_tier("xhigh"), effort_tier("max"));
        assert!(effort_tier("xhigh") > effort_tier("high"));
        assert_eq!(effort_tier("bogus"), None);
    }

    #[test]
    fn exact_map_hit_is_byte_identical_to_legacy() {
        // deepseek live map
        let p = provider(&[
            ("low", "high"),
            ("medium", "high"),
            ("high", "high"),
            ("max", "max"),
            ("xhigh", "max"),
        ]);
        assert_eq!(resolve_effort("xhigh", &p), "max");
        assert_eq!(resolve_effort("low", &p), "high");
        assert_eq!(resolve_effort("max", &p), "max");
    }

    #[test]
    fn inbound_max_passes_through_when_upstream_supports_max_without_key() {
        // glm-5.3-flash style: supports {none,minimal,low,medium,high,max} but
        // the map has no "max" KEY (only xhigh→max). Inbound max must pass
        // through as max (same-tier member of the supported set).
        let p = provider(&[
            ("none", "none"),
            ("minimal", "minimal"),
            ("low", "low"),
            ("medium", "medium"),
            ("high", "high"),
            ("xhigh", "max"),
        ]);
        assert_eq!(resolve_effort("max", &p), "max");
        assert_eq!(resolve_effort("xhigh", &p), "max");
        assert_eq!(resolve_effort("medium", &p), "medium");
    }

    #[test]
    fn missing_level_falls_to_nearest_lower_tier() {
        // deepseek has no medium SUPPORTED value ({high,max}); inbound medium
        // must fall DOWN to high, never up.
        let p = provider(&[
            ("low", "high"),
            ("medium", "high"),
            ("high", "high"),
            ("max", "max"),
            ("xhigh", "max"),
        ]);
        assert_eq!(resolve_effort("medium", &p), "high");
        assert_eq!(resolve_effort("minimal", &p), "high");
    }

    #[test]
    fn kimi_medium_maps_low_via_explicit_entry() {
        let p = provider(&[
            ("low", "low"),
            ("medium", "low"),
            ("high", "high"),
            ("max", "max"),
            ("xhigh", "max"),
        ]);
        assert_eq!(resolve_effort("medium", &p), "low");
        assert_eq!(resolve_effort("max", &p), "max");
    }

    #[test]
    fn unknown_inbound_word_uses_legacy_high_fallback() {
        let p = provider(&[("high", "high"), ("xhigh", "max")]);
        assert_eq!(resolve_effort("turbo", &p), "high");
    }

    #[test]
    fn degenerate_empty_map_falls_back_high() {
        let p = provider(&[]);
        assert_eq!(resolve_effort("xhigh", &p), "high");
    }
}
