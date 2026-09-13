//! URI passwords are decoded only into the secret store; persisted URI templates
//! keep a placeholder. Substitution encodes the password as URI userinfo.
use crate::{Error, Result};

pub fn encode_password(password: &str) -> String {
    password
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

pub fn extract_password(uri: &str) -> Result<Option<(String, String)>> {
    let Some(scheme) = uri.find("://") else {
        return Ok(None);
    };
    let start = scheme + 3;
    let end = uri[start..]
        .find(['/', '?', '#'])
        .map_or(uri.len(), |n| start + n);
    let Some(at) = uri[start..end].rfind('@').map(|n| start + n) else {
        return Ok(None);
    };
    let Some(colon) = uri[start..at].find(':').map(|n| start + n) else {
        return Ok(None);
    };
    let raw = &uri[colon + 1..at];
    if raw == "{secret}" {
        return Ok(None);
    }
    let mut decoded = Vec::new();
    let mut bytes = raw.as_bytes().iter().copied();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hi = bytes.next().and_then(|c| (c as char).to_digit(16));
            let lo = bytes.next().and_then(|c| (c as char).to_digit(16));
            match (hi, lo) {
                (Some(a), Some(b)) => decoded.push((a * 16 + b) as u8),
                _ => {
                    return Err(Error::Invalid(
                        "invalid percent encoding in URI password".into(),
                    ))
                }
            }
        } else {
            decoded.push(b)
        }
    }
    let password = String::from_utf8(decoded)
        .map_err(|_| Error::Invalid("URI password must be UTF-8".into()))?;
    if password == "{secret}" {
        return Ok(None);
    }
    Ok(Some((
        format!("{}{{secret}}{}", &uri[..colon + 1], &uri[at..]),
        password,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_replica_topology_and_query_and_roundtrips_reserved_password() {
        let uri =
            "mongodb://alice:p%40ss%3A%2F%25@one:27017,two:27018/db?authSource=admin&tls=true";
        let (template, password) = extract_password(uri).unwrap().unwrap();
        assert_eq!(password, "p@ss:/%");
        assert!(!template.contains("p%40"));
        assert_eq!(
            template.replace("{secret}", &encode_password(&password)),
            uri
        );
        assert!(extract_password(&template).unwrap().is_none());
    }
    #[test]
    fn rejects_invalid_encoding_without_echoing_secret() {
        let error = extract_password("mongodb://u:sensitive%XX@host/db")
            .unwrap_err()
            .to_string();
        assert!(!error.contains("sensitive"));
    }
}
