//! Browser-provided identity paired with an embedded login session.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserIdentity {
    pub user_agent: String,
    #[serde(default)]
    pub client_hints: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<BrowserEnvironment>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserEnvironment {
    pub platform: String,
    pub language: String,
    pub timezone: String,
    pub screen_width: u32,
    pub screen_height: u32,
    pub hardware_concurrency: Option<u32>,
    pub device_memory: Option<f64>,
}

impl BrowserIdentity {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.user_agent.trim().is_empty()
                && self.user_agent.len() <= 2048
                && self.user_agent.bytes().all(|b| (32..=126).contains(&b)),
            "CONFIG_INVALID"
        );
        for (name, value) in &self.client_hints {
            ensure!(
                matches!(
                    name.as_str(),
                    "sec-ch-ua"
                        | "sec-ch-ua-mobile"
                        | "sec-ch-ua-platform"
                        | "sec-ch-ua-platform-version"
                        | "sec-ch-ua-arch"
                        | "sec-ch-ua-bitness"
                        | "sec-ch-ua-full-version-list"
                ),
                "CONFIG_INVALID"
            );
            ensure!(
                !value.is_empty()
                    && value.len() <= 2048
                    && value.bytes().all(|b| (32..=126).contains(&b)),
                "CONFIG_INVALID"
            );
        }
        if let Some(env) = &self.environment {
            for text in [&env.platform, &env.language, &env.timezone] {
                ensure!(
                    text.len() <= 128 && text.bytes().all(|b| (32..=126).contains(&b)),
                    "CONFIG_INVALID"
                );
            }
            ensure!(
                env.screen_width <= 32768
                    && env.screen_height <= 32768
                    && env
                        .hardware_concurrency
                        .is_none_or(|n| (1..=1024).contains(&n))
                    && env
                        .device_memory
                        .is_none_or(|n| n.is_finite() && n > 0.0 && n <= 1024.0),
                "CONFIG_INVALID"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_injected_headers_and_accepts_native_browser_fields() {
        let mut identity: BrowserIdentity = serde_json::from_value(serde_json::json!({
            "user_agent":"Mozilla/5.0 Native browser", "client_hints":{"sec-ch-ua-platform":"\"Windows\""},
            "environment":{"platform":"Win32","language":"zh-CN","timezone":"Asia/Shanghai",
                "screen_width":2560,"screen_height":1440,"hardware_concurrency":16,"device_memory":8}
        })).unwrap();
        identity.validate().unwrap();
        identity
            .client_hints
            .insert("cookie".into(), "secret".into());
        assert!(identity.validate().is_err());
        identity.client_hints.remove("cookie");
        identity
            .client_hints
            .insert("sec-ch-ua".into(), "browser\r\nCookie: secret".into());
        assert!(identity.validate().is_err());
        identity.client_hints.clear();
        identity.environment.as_mut().unwrap().screen_width = 100_000;
        assert!(identity.validate().is_err());
    }
}
