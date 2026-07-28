//! Honest single-harness detection and profile routing.
//!
//! Registry keys are profiles, not necessarily distinct harnesses. A user can
//! register several model/account profiles for one adapter, so diversity is
//! counted by adapter family. Multiple profiles in one family may be routed
//! sequentially, but they never become artificial cross-harness diversity.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::adapter::locate_executable;
use crate::bench::{HarnessConfig, HarnessRegistry};
use crate::invocation::resolve_worker_invocation;
use crate::probe::probed_from;

/// Exact launch notice required by the V1 single-harness specification.
pub const SINGLE_HARNESS_MESSAGE: &str = "One capable harness detected. Parallel cross-harness deliberation is unavailable. Running a sequential plan with self-review.";

/// One explicitly configured provider/model profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelProfile {
    /// Registry key used to route work.
    pub key: String,
    /// Configured provider name.
    pub provider: String,
    /// Configured model name.
    pub model: String,
}

/// One explicitly labeled account profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountProfile {
    /// Registry key used to route work.
    pub key: String,
    /// User-supplied account identity label.
    pub account: String,
}

/// The one capable adapter family and its usable profiles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SingleHarnessPlan {
    /// Adapter family; multiple registry keys with this adapter are one harness.
    pub adapter: String,
    /// Available profiles that can host the conductor.
    pub brain_profiles: Vec<String>,
    /// Verified profiles that can execute and review bounded work.
    pub worker_profiles: Vec<String>,
    /// Explicit provider/model profiles in this family.
    pub models: Vec<ModelProfile>,
    /// Explicit account profiles in this family.
    pub accounts: Vec<AccountProfile>,
}

impl SingleHarnessPlan {
    /// Whether the registry explicitly exposes more than one distinct model.
    #[must_use]
    pub fn has_multiple_models(&self) -> bool {
        self.models
            .iter()
            .map(|profile| (&profile.provider, &profile.model))
            .collect::<BTreeSet<_>>()
            .len()
            > 1
    }

    /// Whether the registry explicitly exposes more than one account label.
    #[must_use]
    pub fn has_multiple_accounts(&self) -> bool {
        self.accounts
            .iter()
            .map(|profile| &profile.account)
            .collect::<BTreeSet<_>>()
            .len()
            > 1
    }
}

/// Extract the explicit `--provider <p> --model <m>` profile shape written by
/// `pio harness add`.
#[must_use]
pub fn provider_model_args(config: &HarnessConfig) -> Option<(&str, &str)> {
    let mut iter = config.args.iter();
    let mut provider = None;
    let mut model = None;
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--provider" => provider = iter.next().map(String::as_str),
            "--model" => model = iter.next().map(String::as_str),
            _ => {}
        }
    }
    provider.zip(model)
}

/// Read an explicit account identity from the registry's additive profile
/// metadata. The label is routing identity only: the user-configured profile
/// command/args remain responsible for selecting that account.
#[must_use]
pub fn account_label(config: &HarnessConfig) -> Option<&str> {
    config
        .extra
        .get("account")
        .and_then(serde_json::Value::as_str)
        .filter(|account| !account.trim().is_empty())
}

fn brain_available(config: &HarnessConfig) -> bool {
    config.roles.iter().any(|role| role == "brain") && locate_executable(&config.command).is_some()
}

/// Whether one configured profile can receive bounded worker dispatches.
#[must_use]
pub fn worker_capable(
    registry: &HarnessRegistry,
    config: &HarnessConfig,
    cwd: Option<&Path>,
) -> bool {
    config.roles.iter().any(|role| role == "worker")
        && locate_executable(&config.command).is_some()
        && resolve_worker_invocation(config, &probed_from(registry, &config.adapter), cwd).is_ok()
}

/// Detect exactly one capable adapter family.
///
/// A family qualifies only when it has an available brain profile and a
/// verified worker profile. Multiple registered profiles for the same adapter
/// remain one harness and are retained for sequential model routing.
#[must_use]
pub fn detect(registry: &HarnessRegistry, cwd: Option<&Path>) -> Option<SingleHarnessPlan> {
    let mut brains: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut workers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (key, config) in &registry.harnesses {
        if brain_available(config) {
            brains
                .entry(config.adapter.clone())
                .or_default()
                .push(key.clone());
        }
        if worker_capable(registry, config, cwd) {
            workers
                .entry(config.adapter.clone())
                .or_default()
                .push(key.clone());
        }
    }
    let families = brains
        .keys()
        .filter(|adapter| workers.contains_key(*adapter))
        .cloned()
        .collect::<Vec<_>>();
    let [adapter] = families.as_slice() else {
        return None;
    };
    let brain_profiles = brains.remove(adapter).unwrap_or_default();
    let worker_profiles = workers.remove(adapter).unwrap_or_default();
    let models = worker_profiles
        .iter()
        .filter_map(|key| {
            let config = registry.harnesses.get(key)?;
            let (provider, model) = provider_model_args(config)?;
            Some(ModelProfile {
                key: key.clone(),
                provider: provider.to_owned(),
                model: model.to_owned(),
            })
        })
        .collect();
    let accounts = worker_profiles
        .iter()
        .filter_map(|key| {
            let config = registry.harnesses.get(key)?;
            Some(AccountProfile {
                key: key.clone(),
                account: account_label(config)?.to_owned(),
            })
        })
        .collect();
    Some(SingleHarnessPlan {
        adapter: adapter.clone(),
        brain_profiles,
        worker_profiles,
        models,
        accounts,
    })
}

/// Whether two registry profiles are proven to be different harness families.
///
/// Missing metadata degrades conservatively to "not distinct"; only two known,
/// different adapter families can support an independent-diversity claim.
#[must_use]
pub fn distinct_families(registry: &HarnessRegistry, left: &str, right: &str) -> bool {
    let Some(left) = registry.harnesses.get(left) else {
        return false;
    };
    let Some(right) = registry.harnesses.get(right) else {
        return false;
    };
    left.adapter != right.adapter
}

/// Pick another profile only when one harness family explicitly lists more
/// than one distinct model or user-supplied account label.
#[must_use]
pub fn alternate_profile(
    registry: &HarnessRegistry,
    executor: &str,
    candidates: &[String],
) -> Option<String> {
    let executor_config = registry.harnesses.get(executor)?;
    let family = &executor_config.adapter;
    let modeled = candidates
        .iter()
        .filter_map(|key| {
            let config = registry.harnesses.get(key)?;
            (config.adapter == *family)
                .then(|| provider_model_args(config).map(|model| (key, model)))
                .flatten()
        })
        .collect::<Vec<_>>();
    let distinct = modeled
        .iter()
        .map(|(_, (provider, model))| (*provider, *model))
        .collect::<BTreeSet<_>>();
    if distinct.len() > 1 {
        let executor_model = provider_model_args(executor_config);
        if let Some(key) = modeled
            .into_iter()
            .find(|(key, model)| key.as_str() != executor && Some(*model) != executor_model)
            .map(|(key, _)| key.clone())
        {
            return Some(key);
        }
    }
    let accounted = candidates
        .iter()
        .filter_map(|key| {
            let config = registry.harnesses.get(key)?;
            (config.adapter == *family)
                .then(|| account_label(config).map(|account| (key, account)))
                .flatten()
        })
        .collect::<Vec<_>>();
    let distinct = accounted
        .iter()
        .map(|(_, account)| *account)
        .collect::<BTreeSet<_>>();
    if distinct.len() <= 1 {
        return None;
    }
    let executor_account = account_label(executor_config);
    accounted
        .into_iter()
        .find(|(key, account)| key.as_str() != executor && Some(*account) != executor_account)
        .map(|(key, _)| key.clone())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn profile(adapter: &str, model: Option<&str>) -> HarnessConfig {
        HarnessConfig {
            command: "/bin/sh".to_owned(),
            args: model.map_or_else(Vec::new, |model| {
                vec![
                    "--provider".to_owned(),
                    "fixture".to_owned(),
                    "--model".to_owned(),
                    model.to_owned(),
                ]
            }),
            resume_args: Vec::new(),
            roles: vec!["brain".to_owned(), "worker".to_owned()],
            adapter: adapter.to_owned(),
            dispatch_args: vec!["-c".to_owned()],
            dispatch_uses_stdin: false,
            dispatch_timeout_sec: 30,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn multiple_profiles_of_one_adapter_are_one_harness() {
        let mut registry = HarnessRegistry {
            harnesses: BTreeMap::from([
                ("solo-a".to_owned(), profile("solo", Some("a"))),
                ("solo-b".to_owned(), profile("solo", Some("b"))),
            ]),
            ..HarnessRegistry::default()
        };
        let plan = detect(&registry, None).expect("one harness family");
        assert_eq!(plan.adapter, "solo");
        assert_eq!(plan.worker_profiles, ["solo-a", "solo-b"]);
        assert!(plan.has_multiple_models());
        assert!(!plan.has_multiple_accounts());

        registry
            .harnesses
            .insert("other".to_owned(), profile("other", None));
        assert!(
            detect(&registry, None).is_none(),
            "two adapter families must not enter single-harness mode"
        );
    }

    #[test]
    fn alternate_model_routing_requires_two_distinct_models() {
        let mut registry = HarnessRegistry {
            harnesses: BTreeMap::from([
                ("solo-a".to_owned(), profile("solo", Some("a"))),
                ("solo-a-copy".to_owned(), profile("solo", Some("a"))),
            ]),
            ..HarnessRegistry::default()
        };
        let candidates = vec!["solo-a".to_owned(), "solo-a-copy".to_owned()];
        assert_eq!(
            alternate_profile(&registry, "solo-a", &candidates),
            None,
            "duplicate profile keys must not manufacture multi-model routing"
        );

        registry
            .harnesses
            .insert("solo-b".to_owned(), profile("solo", Some("b")));
        let candidates = vec![
            "solo-a".to_owned(),
            "solo-a-copy".to_owned(),
            "solo-b".to_owned(),
        ];
        assert_eq!(
            alternate_profile(&registry, "solo-a", &candidates).as_deref(),
            Some("solo-b")
        );
        assert!(
            !distinct_families(&registry, "solo-a", "solo-b"),
            "different models of one adapter are not independent harnesses"
        );
    }

    #[test]
    fn alternate_account_routing_requires_two_explicit_account_labels() {
        let mut first = profile("solo", Some("a"));
        first
            .extra
            .insert("account".to_owned(), serde_json::json!("personal"));
        let mut second = profile("solo", Some("a"));
        second
            .extra
            .insert("account".to_owned(), serde_json::json!("work"));
        let registry = HarnessRegistry {
            harnesses: BTreeMap::from([
                ("solo-personal".to_owned(), first),
                ("solo-work".to_owned(), second),
            ]),
            ..HarnessRegistry::default()
        };
        let candidates = vec!["solo-personal".to_owned(), "solo-work".to_owned()];
        assert_eq!(
            alternate_profile(&registry, "solo-personal", &candidates).as_deref(),
            Some("solo-work")
        );
        let plan = detect(&registry, None).expect("one harness family");
        assert!(!plan.has_multiple_models());
        assert!(plan.has_multiple_accounts());
    }
}
