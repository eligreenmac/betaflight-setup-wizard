// SPDX-License-Identifier: GPL-3.0-or-later
//! Wire types for the Betaflight cloud build API (https://build.betaflight.com), as used by
//! Betaflight Configurator `src/js/BuildApi.ts` (GPL-3.0-or-later). Unknown fields are kept in
//! `extra` where the shape is loose, so API additions don't break deserialisation.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Entry of `GET /api/targets`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetSummary {
    pub target: String,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `GET /api/targets/{target}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetReleases {
    #[serde(default)]
    pub target: Option<String>,
    pub releases: Vec<Release>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub release: String,
    #[serde(default)]
    pub label: Option<String>,
    /// "Stable", "ReleaseCandidate", "Unstable", ...
    #[serde(rename = "type", default)]
    pub release_type: Option<String>,
    #[serde(default)]
    pub withdrawn: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Release {
    pub fn is_stable(&self) -> bool {
        self.release_type
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case("stable"))
    }
}

/// `GET /api/builds/{release}/{target}`: target config, cloud-build flag, etc. Kept loose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetDetail {
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub release: Option<String>,
    #[serde(default)]
    pub cloud_build: Option<bool>,
    #[serde(default)]
    pub mcu: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `GET /api/options/{release}`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildOptions {
    #[serde(default)]
    pub radio_protocols: Vec<ApiBuildOption>,
    #[serde(default)]
    pub telemetry_protocols: Vec<ApiBuildOption>,
    #[serde(default)]
    pub motor_protocols: Vec<ApiBuildOption>,
    #[serde(default)]
    pub general_options: Vec<ApiBuildOption>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiBuildOption {
    pub name: String,
    /// The define sent in `BuildRequest::options`, e.g. "USE_SERIALRX_CRSF".
    pub value: String,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub grouped_name: Option<String>,
    #[serde(default)]
    pub includes_telemetry: bool,
}

/// `POST /api/builds` body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildRequest {
    pub target: String,
    pub release: String,
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Client tag (the configurator sends `client: "configurator"` / version info).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
}

/// `POST /api/builds` reply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildResponse {
    pub key: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
}

/// `GET /api/builds/{key}/status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildStatus {
    /// "queued" | "success" | "failure" | ...
    pub status: String,
    /// Server-suggested total wait, in seconds.
    #[serde(default)]
    pub time_out: Option<u64>,
    #[serde(default)]
    pub configuration: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl BuildStatus {
    pub fn is_queued(&self) -> bool {
        self.status == "queued"
    }
    pub fn is_success(&self) -> bool {
        self.status == "success"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialises_api_shapes() {
        let r: TargetReleases = serde_json::from_str(
            r#"{"target":"STM32F405","releases":[{"release":"4.5.1","label":"4.5.1","type":"Stable"},
                {"release":"4.6.0-RC1","type":"ReleaseCandidate","withdrawn":true,"date":"x"}]}"#,
        )
        .unwrap();
        assert!(r.releases[0].is_stable());
        assert!(r.releases[1].withdrawn);
        assert_eq!(r.releases[1].extra["date"], "x");

        let o: BuildOptions = serde_json::from_str(
            r#"{"radioProtocols":[{"name":"CRSF","value":"USE_SERIALRX_CRSF","default":true,"includesTelemetry":true}],
                "generalOptions":[{"name":"LED","value":"USE_LED_STRIP","group":"Misc","groupedName":"Misc: LED"}]}"#,
        )
        .unwrap();
        assert!(o.radio_protocols[0].default && o.radio_protocols[0].includes_telemetry);
        assert_eq!(
            o.general_options[0].grouped_name.as_deref(),
            Some("Misc: LED")
        );
        assert!(o.motor_protocols.is_empty());

        let s: BuildStatus = serde_json::from_str(r#"{"status":"queued","timeOut":240}"#).unwrap();
        assert!(s.is_queued());
        assert_eq!(s.time_out, Some(240));

        let req = BuildRequest {
            target: "T".into(),
            release: "4.5.1".into(),
            options: vec!["A".into()],
            commit: None,
            client: None,
        };
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            r#"{"target":"T","release":"4.5.1","options":["A"]}"#
        );
    }
}
