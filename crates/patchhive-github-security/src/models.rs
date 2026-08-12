use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GitHubDependabotAlert {
    #[serde(default)]
    pub number: u32,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub dependency: GitHubAlertDependency,
    #[serde(default)]
    pub security_advisory: GitHubSecurityAdvisory,
    #[serde(default)]
    pub security_vulnerability: GitHubSecurityVulnerability,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GitHubAlertDependency {
    #[serde(default)]
    pub package: GitHubPackageRef,
    #[serde(default)]
    pub manifest_path: String,
    #[serde(default)]
    pub scope: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GitHubSecurityAdvisory {
    #[serde(default)]
    pub ghsa_id: String,
    #[serde(default)]
    pub cve_id: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub cwes: Vec<GitHubCweRef>,
    #[serde(default)]
    pub references: Vec<GitHubReference>,
    pub epss: Option<GitHubEpss>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GitHubSecurityVulnerability {
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub vulnerable_version_range: String,
    #[serde(default)]
    pub package: GitHubPackageRef,
    pub first_patched_version: Option<GitHubPatchedVersion>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GitHubPackageRef {
    #[serde(default)]
    pub ecosystem: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GitHubPatchedVersion {
    #[serde(default)]
    pub identifier: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GitHubCweRef {
    #[serde(default)]
    pub cwe_id: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GitHubReference {
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GitHubEpss {
    #[serde(default)]
    pub percentage: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GitHubCodeScanningAlert {
    #[serde(default)]
    pub number: u32,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub rule: GitHubCodeRule,
    #[serde(default)]
    pub tool: GitHubCodeTool,
    #[serde(default)]
    pub most_recent_instance: GitHubCodeInstance,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GitHubCodeRule {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub security_severity_level: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GitHubCodeTool {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GitHubCodeInstance {
    #[serde(default, rename = "ref")]
    pub ref_: String,
    #[serde(default)]
    pub message: GitHubCodeMessage,
    #[serde(default)]
    pub location: GitHubCodeLocation,
    #[serde(default)]
    pub classifications: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GitHubCodeMessage {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GitHubCodeLocation {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub start_line: u32,
}

#[cfg(test)]
mod tests {
    use super::{GitHubCodeScanningAlert, GitHubDependabotAlert};
    use serde_json::json;

    #[test]
    fn dependabot_alert_preserves_nested_security_evidence() {
        let alert: GitHubDependabotAlert = serde_json::from_value(json!({
            "number": 17,
            "html_url": "https://github.com/patchhive/tendwright/security/dependabot/17",
            "created_at": "2026-08-11T12:00:00Z",
            "updated_at": "2026-08-12T12:00:00Z",
            "dependency": {
                "package": { "ecosystem": "cargo", "name": "example-crate" },
                "manifest_path": "Cargo.toml",
                "scope": "runtime"
            },
            "security_advisory": {
                "ghsa_id": "GHSA-1234-5678-9012",
                "cve_id": "CVE-2026-12345",
                "summary": "Representative advisory",
                "severity": "high",
                "cwes": [{ "cwe_id": "CWE-79" }],
                "references": [{ "url": "https://example.test/advisory" }],
                "epss": { "percentage": 0.42 }
            },
            "security_vulnerability": {
                "severity": "high",
                "vulnerable_version_range": "< 2.0.0",
                "package": { "ecosystem": "cargo", "name": "example-crate" },
                "first_patched_version": { "identifier": "2.0.0" }
            }
        }))
        .expect("representative Dependabot alert should decode");

        assert_eq!(alert.number, 17);
        assert_eq!(alert.dependency.package.name, "example-crate");
        assert_eq!(alert.security_advisory.cwes[0].cwe_id, "CWE-79");
        assert_eq!(
            alert
                .security_advisory
                .epss
                .expect("EPSS should be present")
                .percentage,
            0.42
        );
        assert_eq!(
            alert
                .security_vulnerability
                .first_patched_version
                .expect("patched version should be present")
                .identifier,
            "2.0.0"
        );
    }

    #[test]
    fn code_scanning_alert_preserves_ref_and_location() {
        let alert: GitHubCodeScanningAlert = serde_json::from_value(json!({
            "number": 9,
            "rule": {
                "id": "rust/sql-injection",
                "name": "SQL injection",
                "description": "Untrusted SQL input",
                "severity": "error",
                "security_severity_level": "high",
                "tags": ["security", "external/cwe/cwe-89"]
            },
            "tool": { "name": "CodeQL" },
            "most_recent_instance": {
                "ref": "refs/heads/main",
                "message": { "text": "Query built from untrusted input" },
                "location": { "path": "src/db.rs", "start_line": 41 },
                "classifications": ["test"]
            }
        }))
        .expect("representative code-scanning alert should decode");

        assert_eq!(alert.rule.security_severity_level, "high");
        assert_eq!(alert.tool.name, "CodeQL");
        assert_eq!(alert.most_recent_instance.ref_, "refs/heads/main");
        assert_eq!(alert.most_recent_instance.location.path, "src/db.rs");
        assert_eq!(alert.most_recent_instance.location.start_line, 41);
    }

    #[test]
    fn optional_security_fields_remain_absent_when_github_omits_them() {
        let alert: GitHubDependabotAlert = serde_json::from_value(json!({
            "security_advisory": {},
            "security_vulnerability": {}
        }))
        .expect("partial alert should retain explicit optional absence");

        assert!(alert.security_advisory.epss.is_none());
        assert!(alert.security_vulnerability.first_patched_version.is_none());
    }
}
