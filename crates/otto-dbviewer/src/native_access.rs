//! Native credential ceilings for governed database execution.
use otto_core::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeGrant {
    pub child: String,
    pub operation: &'static str,
}

pub fn setup_error(reason: &str) -> Error {
    Error::Forbidden(format!("native_scope_required: {reason}; configure an administrator-provisioned restricted credential profile"))
}

/// Parse SHOW GRANTS output conservatively. Role/proxy/dynamic/global grants,
/// wildcards and delegation cannot prove an exact native ceiling and are refused.
/// This parses server-produced grants, never classifies caller SQL for security.
pub fn mysql_grants(rows: &[String]) -> Result<Vec<NativeGrant>> {
    if rows.is_empty() {
        return Err(setup_error(
            "native privilege inspection returned no grants",
        ));
    }
    let mut out = Vec::new();
    for row in rows {
        let (privileges, target) = row
            .strip_prefix("GRANT ")
            .and_then(|s| s.split_once(" ON "))
            .ok_or_else(|| {
                setup_error("native roles or unrecognized grant format are unsupported")
            })?;
        let (scope, principal) = target
            .split_once(" TO ")
            .ok_or_else(|| setup_error("unrecognized native grant target"))?;
        if principal.contains("WITH GRANT OPTION") {
            return Err(setup_error("native credential has grant authority"));
        }
        if privileges == "USAGE" && scope == "*.*" {
            continue;
        }
        let child = scope.strip_prefix('`').and_then(|s| s.strip_suffix("`.*"))
            .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
            .ok_or_else(|| setup_error("only exact simple database-level native grants are supported (no wildcard/global/table grants)"))?;
        for privilege in privileges.split(", ") {
            let operation = match privilege {
                "SELECT" => "db_query",
                "INSERT" | "UPDATE" | "DELETE" => "db_data",
                "CREATE" | "ALTER" | "DROP" | "INDEX" | "REFERENCES" | "TRIGGER" => "db_schema",
                "SHOW VIEW" => "db_browse",
                _ => return Err(setup_error("native credential has unsupported administrative, routine, or session privileges")),
            };
            let grant = NativeGrant {
                child: child.into(),
                operation,
            };
            if !out.contains(&grant) {
                out.push(grant);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_rejects_global_privileges_roles_grant_option_and_wildcards() {
        for grant in [
            "GRANT ALL PRIVILEGES ON *.* TO `u`@`%`",
            "GRANT SELECT ON *.* TO `u`@`%`",
            "GRANT `reader`@`%` TO `u`@`%`",
            "GRANT SELECT ON `shop`.* TO `u`@`%` WITH GRANT OPTION",
            "GRANT SELECT ON `shop%`.* TO `u`@`%`",
            "GRANT EXECUTE ON `shop`.* TO `u`@`%`",
            "GRANT PROXY ON ''@'' TO `u`@`%`",
        ] {
            assert!(mysql_grants(&[grant.into()]).is_err(), "accepted {grant}");
        }
    }

    #[test]
    fn mysql_maps_exact_database_grants_to_separate_data_schema_and_query_rights() {
        let grants = mysql_grants(&[
            "GRANT USAGE ON *.* TO `u`@`%`".into(),
            "GRANT SELECT, INSERT, UPDATE, DELETE, CREATE, ALTER, DROP, INDEX ON `shop`.* TO `u`@`%`".into(),
        ]).unwrap();
        assert!(grants.contains(&NativeGrant {
            child: "shop".into(),
            operation: "db_query"
        }));
        assert!(grants.contains(&NativeGrant {
            child: "shop".into(),
            operation: "db_data"
        }));
        assert!(grants.contains(&NativeGrant {
            child: "shop".into(),
            operation: "db_schema"
        }));
    }
}
