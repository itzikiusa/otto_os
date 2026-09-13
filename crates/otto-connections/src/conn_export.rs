//! Portable connection exports. Native formats are verified against vendor
//! documentation/source linked in docs/features/connections-ssh-sftp.md.

use otto_core::{
    connection_credentials::{encode_password, extract_password},
    Error, Result,
};
use serde::Serialize;
use serde_json::{json, Value};

// Deliberately no Debug: these transient values can contain opted-in secrets.
#[derive(Clone)]
pub struct ExportProfile {
    pub record: Value,
    pub password: Option<String>,
}
#[derive(Serialize)]
pub struct ExportFormat {
    pub id: &'static str,
    pub label: &'static str,
    pub kinds: Vec<&'static str>,
    pub password_support: &'static str,
    pub description: &'static str,
    pub import_instructions: &'static str,
}
#[derive(Serialize)]
pub struct ExportFile {
    pub name: String,
    pub mime: String,
    pub content: String,
}
#[derive(Serialize)]
pub struct SkippedProfile {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub reason: String,
}
#[derive(Serialize)]
pub struct ExportResult {
    pub format: String,
    pub total_connections: usize,
    pub exported_connections: usize,
    pub contains_passwords: bool,
    pub files: Vec<ExportFile>,
    pub skipped: Vec<SkippedProfile>,
    pub warnings: Vec<String>,
    pub import_instructions: String,
}

pub fn formats() -> Vec<ExportFormat> {
    vec![
        ExportFormat { id:"json",label:"Otto JSON — all connections",kinds:vec!["*"],password_support:"native",description:"Complete profile records, including advanced parameters and optional plaintext credentials.",import_instructions:"JSON format otto-connections, version 1. Each connections entry preserves the profile fields; password is optional. Local paths remain references; private-key files are not read or copied. This is a portable interchange file, not a native third-party backup." },
        ExportFormat { id:"csv",label:"CSV — all connections",kinds:vec!["*"],password_support:"native",description:"Readable summary columns plus record_json for lossless advanced profile data.",import_instructions:"UTF-8 RFC 4180 CSV. Parse record_json in each row as the complete profile record, including password when selected. Summary columns are JSON string literals, so spreadsheet formula prefixes cannot execute. Native tool imports use the dedicated formats below." },
        ExportFormat { id:"mysql_workbench",label:"MySQL Workbench",kinds:vec!["mysql"],password_support:"sidecar",description:"Workbench connections.xml; optional separate credential JSON because Workbench keeps passwords in its OS vault.",import_instructions:"Quit Workbench. Back up its existing connections.xml, then merge the exported db.mgmt.Connection entries into that file (do not overwrite existing connections). Restart Workbench. Enter sidecar passwords into Store in Vault manually; the sidecar is not a Workbench credential import. Profiles requiring inline certificate material or SSH transport are listed as skipped." },
        ExportFormat { id:"dbeaver_mysql",label:"DBeaver — MySQL",kinds:vec!["mysql"],password_support:"native",description:"DBeaver Custom CSV with MySQL JDBC URLs and optional passwords.",import_instructions:"DBeaver: File → Import → Third Party Configuration → Custom. Select the MySQL driver, CSV and UTF-8, then choose this file. Driver availability depends on your DBeaver installation. Profiles requiring SSH transport or inline certificates are skipped." },
        ExportFormat { id:"dbeaver_mongodb",label:"DBeaver — MongoDB",kinds:vec!["mongodb"],password_support:"native",description:"DBeaver Custom CSV with MongoDB connection URIs and optional passwords.",import_instructions:"DBeaver: File → Import → Third Party Configuration → Custom. Select the MongoDB driver, CSV and UTF-8, then choose this file. MongoDB requires a DBeaver edition with that driver. Profiles requiring SSH transport or inline certificates are skipped." },
        ExportFormat { id:"nosqlbooster",label:"NoSQLBooster 9+ — MongoDB",kinds:vec!["mongodb"],password_support:"native",description:"The native MongoDB URI-list import supported since NoSQLBooster 9.",import_instructions:"NoSQLBooster 9+: Connections → Import → From File...; alternatively copy the text and choose From Clipboard (URI List). Each URI is preceded by a // comment naming the connection. Passwords are percent-encoded when selected; otherwise enter them after import. SSH and certificate settings that cannot be represented by a URI are listed as skipped." },
        ExportFormat { id:"redisinsight",label:"RedisInsight — Redis",kinds:vec!["redis"],password_support:"native",description:"Native RedisInsight JSON array with database, ACL username and TLS settings.",import_instructions:"RedisInsight: Add Redis database → Import connections, then choose the JSON file. Supports direct standalone Redis profiles, including inline TLS certificates. SSH and sentinel/cluster configurations without a verified equivalent are listed as skipped. Private-key files are never read." },
    ]
}

fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn stringify(value: &impl Serialize) -> Result<String> {
    serde_json::to_string_pretty(value)
        .map_err(|_| invalid("Could not serialize connection export"))
}
fn field(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}
fn database(params: &Value) -> String {
    let db = field(params, "db");
    if db.is_empty() {
        field(params, "database")
    } else {
        db
    }
}
fn sensitive(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "secret",
        "token",
        "credential",
        "authorization",
        "cookie",
        "private_key",
        "privatekey",
        "client_key",
        "apikey",
        "api_key",
        "access_key",
    ]
    .iter()
    .any(|s| k.contains(s))
}
fn opaque_reference(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.contains("keychain") || (sensitive(&k) && k.ends_with("_ref"))
}
fn decode_key(raw: &str) -> Option<String> {
    let mut output = Vec::new();
    let mut bytes = raw.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hi = (bytes.next()? as char).to_digit(16)?;
            let lo = (bytes.next()? as char).to_digit(16)?;
            output.push((hi * 16 + lo) as u8);
        } else {
            output.push(b);
        }
    }
    String::from_utf8(output).ok()
}
fn without_uri_credentials(uri: &str) -> String {
    let mut out = uri.to_string();
    if let Some(scheme) = out.find("://") {
        let start = scheme + 3;
        let end = out[start..]
            .find(['/', '?', '#'])
            .map_or(out.len(), |i| start + i);
        if let Some(at) = out[start..end].rfind('@').map(|i| start + i) {
            if let Some(colon) = out[start..at].find(':').map(|i| start + i) {
                out.replace_range(colon..at, "");
            }
        }
        if let Some(q) = out.find('?') {
            let base = out[..q].to_string();
            let query = out[q + 1..]
                .split('&')
                .filter(|pair| {
                    decode_key(pair.split('=').next().unwrap_or(""))
                        .is_some_and(|key| !sensitive(&key))
                })
                .collect::<Vec<_>>()
                .join("&");
            out = if query.is_empty() {
                base
            } else {
                format!("{base}?{query}")
            };
        }
    }
    out
}
fn sanitize(value: &mut Value, include_passwords: bool) {
    match value {
        Value::Object(map) => {
            if map.contains_key("$secret") {
                map.clear();
                return;
            }
            map.retain(|key, _| !opaque_reference(key) && (include_passwords || !sensitive(key)));
            let sensitive_pair = ["name", "key"]
                .iter()
                .any(|k| map.get(*k).and_then(Value::as_str).is_some_and(sensitive));
            if !include_passwords && sensitive_pair {
                map.remove("value");
            }
            map.values_mut()
                .for_each(|v| sanitize(v, include_passwords));
        }
        Value::Array(values) => values
            .iter_mut()
            .for_each(|v| sanitize(v, include_passwords)),
        Value::String(text) => {
            if text.starts_with("keychain:")
                || text.starts_with("secret:")
                || text.starts_with("otto-keychain:")
            {
                *text = String::new();
            } else if !include_passwords {
                *text = without_uri_credentials(text);
                if text.contains("PRIVATE KEY-----")
                    || text.to_ascii_lowercase().starts_with("bearer ")
                    || text.to_ascii_lowercase().starts_with("basic ")
                {
                    *text = String::new();
                }
            }
        }
        _ => {}
    }
}
fn port(params: &Value, default: u16) -> Result<u16> {
    if params
        .get("port")
        .is_some_and(|v| !v.is_null() && !v.is_string() && !v.is_number())
    {
        return Err(invalid("Invalid connection port"));
    }
    let s = field(params, "port");
    if s.is_empty() {
        return Ok(default);
    }
    s.parse::<u16>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| invalid("Invalid connection port"))
}
fn host(params: &Value) -> Result<String> {
    let h = field(params, "host");
    if h.is_empty() || h.chars().any(char::is_control) || h.contains(['/', '?', '#', '@']) {
        return Err(invalid("A direct hostname is required"));
    }
    Ok(if h.contains(':') && !h.starts_with('[') {
        format!("[{h}]")
    } else {
        h
    })
}
fn tls_mode(params: &Value) -> String {
    if params.get("tls").is_none_or(Value::is_null) && params["secure"] == true {
        "required".into()
    } else {
        field(&params["tls"], "mode")
    }
}
fn tls_enabled(params: &Value) -> bool {
    !matches!(tls_mode(params).as_str(), "" | "disabled" | "disable")
}
fn tls_verify(params: &Value) -> bool {
    params["tls"]["verify"].as_bool().unwrap_or(true)
}
fn native_supported(format: &str, p: &Value) -> Result<()> {
    if !matches!(
        tls_mode(p).as_str(),
        "" | "disabled" | "disable" | "required" | "require" | "preferred" | "prefer"
    ) {
        return Err(invalid("Unrecognized TLS mode requires Otto JSON/CSV"));
    }
    if p.get("tls").is_some_and(|v| !v.is_null() && !v.is_object())
        || p.get("ssl")
            .is_some_and(|v| !v.is_null() && v != &json!(false))
    {
        return Err(invalid(
            "Legacy TLS options require normalization before native export; use Otto JSON/CSV",
        ));
    }
    if !field(p, "jump").is_empty()
        || p.get("ssh")
            .is_some_and(|s| s.is_object() && s["enabled"].as_bool() != Some(false))
    {
        return Err(invalid(
            "SSH transport is not representable in this format; use Otto JSON/CSV",
        ));
    }
    if format != "redisinsight"
        && ["ca_cert", "client_cert", "client_key", "server_name"]
            .iter()
            .any(|k| !field(&p["tls"], k).is_empty())
    {
        return Err(invalid(
            "Inline certificates or custom TLS server names require Otto JSON/CSV",
        ));
    }
    if format == "redisinsight"
        && ["sentinel", "sentinels", "cluster", "cluster_nodes"]
            .iter()
            .any(|k| {
                p.get(*k)
                    .is_some_and(|v| !v.is_null() && v != &json!(false))
            })
    {
        return Err(invalid("Sentinel/cluster transport requires Otto JSON/CSV"));
    }
    Ok(())
}
fn mongo_uri(record: &Value, password: Option<&str>, include: bool) -> Result<String> {
    let p = &record["params"];
    let mut uri = field(p, "conn_string");
    let built_from_fields = uri.is_empty();
    if uri.is_empty() {
        let h = host(p)?;
        let u = field(p, "user");
        let auth = if u.is_empty() {
            String::new()
        } else {
            format!("{}@", encode_password(&u))
        };
        uri = format!(
            "mongodb://{auth}{h}:{}/{}",
            port(p, 27017)?,
            encode_password(&database(p))
        );
    }
    if !uri.starts_with("mongodb://") && !uri.starts_with("mongodb+srv://") {
        return Err(invalid("MongoDB URI must use mongodb:// or mongodb+srv://"));
    }
    if uri.chars().any(char::is_control) {
        return Err(invalid(
            "MongoDB URI contains a line break or control character",
        ));
    }
    if include {
        if uri.contains("{secret}") {
            uri = uri.replace(
                "{secret}",
                &encode_password(
                    password.ok_or_else(|| invalid("Saved MongoDB password is unavailable"))?,
                ),
            );
        } else if let Some(secret) = password {
            if extract_password(&uri)?.is_none() {
                let start = uri.find("://").unwrap() + 3;
                let end = uri[start..]
                    .find(['/', '?', '#'])
                    .map_or(uri.len(), |i| start + i);
                if let Some(at) = uri[start..end].rfind('@').map(|i| start + i) {
                    uri.insert_str(at, &format!(":{}", encode_password(secret)));
                }
            }
        }
    } else {
        uri = without_uri_credentials(&uri);
    }
    // Match the runtime: a full MongoDB URI owns its TLS settings. Only the
    // host/port fallback consumes the separate TLS configuration.
    if built_from_fields && tls_enabled(p) {
        uri.push(if uri.contains('?') { '&' } else { '?' });
        uri.push_str("tls=true");
        if !tls_verify(p) {
            uri.push_str("&tlsAllowInvalidCertificates=true");
        }
    }
    if built_from_fields {
        let mut options = Vec::new();
        if !field(p, "user").is_empty() {
            let source = field(p, "auth_source");
            options.push(format!(
                "authSource={}",
                encode_password(if source.is_empty() { "admin" } else { &source })
            ));
        }
        let replica = field(p, "replica_set");
        if !replica.is_empty() {
            options.push(format!("replicaSet={}", encode_password(&replica)));
        }
        if !options.is_empty() {
            uri.push(if uri.contains('?') { '&' } else { '?' });
            uri.push_str(&options.join("&"));
        }
    }
    Ok(uri)
}
fn csv_row(values: &[String]) -> String {
    values
        .iter()
        .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(",")
        + "\r\n"
}
/// RFC 4180 reader used by portable consumers and round-trip validation. No
/// spreadsheet interpretation: every field is returned byte-for-byte as text.
pub fn parse_csv(input: &str) -> Result<Vec<Vec<String>>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut chars = input.chars().peekable();
    let mut quoted = false;
    let mut closed = false;
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                    closed = true;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() && !closed => quoted = true,
            ',' => {
                row.push(std::mem::take(&mut field));
                closed = false;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                closed = false;
            }
            _ if closed => return Err(invalid("Unexpected content after CSV quote")),
            _ => field.push(c),
        }
    }
    if quoted {
        return Err(invalid("Unterminated CSV quote"));
    }
    if !field.is_empty() || !row.is_empty() || closed {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}
fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn xml_value(key: &str, value: &str, kind: &str) -> String {
    format!(
        "<value type=\"{kind}\" key=\"{}\">{}</value>",
        xml(key),
        xml(value)
    )
}

pub fn render(
    format: &str,
    profiles: Vec<ExportProfile>,
    include_passwords: bool,
) -> Result<ExportResult> {
    let definition = formats()
        .into_iter()
        .find(|f| f.id == format)
        .ok_or_else(|| invalid("Unknown connection export format"))?;
    let mut result = ExportResult {
        format: format.into(),
        total_connections: profiles.len(),
        exported_connections: 0,
        contains_passwords: false,
        files: vec![],
        skipped: vec![],
        warnings: vec![],
        import_instructions: definition.import_instructions.into(),
    };
    let mut records = Vec::new();
    let mut native = Vec::new();
    let mut csv = String::new();
    let mut uri_list = String::new();
    let mut workbench=String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<data grt_format=\"2.0\"><value type=\"list\" content-type=\"object\" content-struct-name=\"db.mgmt.Connection\">\n");
    if format == "csv" {
        csv += &csv_row(&["id", "name", "kind", "workspace_id", "record_json"].map(str::to_string));
    }
    if format.starts_with("dbeaver_") {
        csv += &csv_row(
            &[
                "name", "host", "port", "server", "database", "url", "user", "password", "type",
            ]
            .map(str::to_string),
        );
    }
    for mut profile in profiles {
        let kind = field(&profile.record, "kind");
        let id = field(&profile.record, "id");
        let name = field(&profile.record, "name");
        let operation = (|| -> Result<()> {
            if !definition.kinds.contains(&"*") && !definition.kinds.contains(&kind.as_str()) {
                return Err(invalid("Connection kind is not supported by this target"));
            }
            // First normalize legacy inline userinfo without changing persistent state.
            if let Some(raw) = profile.record["params"]["conn_string"].as_str() {
                match extract_password(raw) {
                    Ok(Some((template, password))) => {
                        profile.record["params"]["conn_string"] = json!(template);
                        if profile.password.is_none() {
                            profile.password = Some(password);
                        }
                    }
                    Err(error) if !matches!(format, "json" | "csv") => return Err(error),
                    _ => {}
                }
            }
            if !matches!(format, "json" | "csv") {
                native_supported(format, &profile.record["params"])?;
            }
            let mut record = profile.record.clone();
            sanitize(&mut record, include_passwords);
            if include_passwords {
                if let Some(password) = &profile.password {
                    record["password"] = json!(password);
                }
            }
            let p = &record["params"];
            match format {
                "json" => {}
                "csv" => {
                    let summaries = ["id", "name", "kind", "workspace_id"]
                        .map(|k| record.get(k).unwrap_or(&Value::Null).to_string());
                    let mut values = summaries.to_vec();
                    values.push(record.to_string());
                    csv += &csv_row(&values);
                }
                "nosqlbooster" => {
                    let uri = mongo_uri(
                        &profile.record,
                        profile.password.as_deref(),
                        include_passwords,
                    )?;
                    uri_list += &format!("// {}\n{uri}\n", name.replace(['\r', '\n'], " "));
                }
                "dbeaver_mysql" | "dbeaver_mongodb" => {
                    let (h, po, database, url, user) = if kind == "mysql" {
                        let h = host(p)?;
                        let po = port(p, 3306)?.to_string();
                        let db = database(p);
                        let mode = match (tls_mode(p).as_str(), tls_verify(p)) {
                            ("required" | "require", true) => "VERIFY_IDENTITY",
                            ("required" | "require", false) => "REQUIRED",
                            ("preferred" | "prefer", _) => "PREFERRED",
                            _ => "DISABLED",
                        };
                        let url = format!(
                            "jdbc:mysql://{h}:{po}/{}?sslMode={mode}",
                            encode_password(&db)
                        );
                        (h, po, db, url, field(p, "user"))
                    } else {
                        (
                            String::new(),
                            String::new(),
                            database(p),
                            mongo_uri(
                                &profile.record,
                                profile.password.as_deref(),
                                include_passwords,
                            )?,
                            field(p, "user"),
                        )
                    };
                    csv += &csv_row(&[
                        name.clone(),
                        h,
                        po,
                        String::new(),
                        database,
                        url,
                        user,
                        if include_passwords {
                            profile.password.clone().unwrap_or_default()
                        } else {
                            String::new()
                        },
                        field(&record, "environment"),
                    ]);
                }
                "mysql_workbench" => {
                    let h = host(p)?.trim_matches(['[', ']']).to_string();
                    let po = port(p, 3306)?.to_string();
                    let mode = match (tls_mode(p).as_str(), tls_verify(p)) {
                        ("required" | "require", true) => "4",
                        ("required" | "require", false) => "2",
                        ("preferred" | "prefer", _) => "1",
                        _ => "0",
                    };
                    workbench+=&format!("<value type=\"object\" struct-name=\"db.mgmt.Connection\" id=\"{}\"><link type=\"object\" struct-name=\"db.mgmt.Driver\" key=\"driver\">com.mysql.rdbms.mysql.driver.native</link>{}<value type=\"dict\" key=\"parameterValues\">",xml(&id),xml_value("name",&name,"string"));
                    for (key, value, ty) in [
                        ("hostName", h.as_str(), "string"),
                        ("port", po.as_str(), "int"),
                        ("userName", field(p, "user").as_str(), "string"),
                        ("schema", database(p).as_str(), "string"),
                        ("useSSL", mode, "int"),
                    ] {
                        workbench += &xml_value(key, value, ty);
                    }
                    workbench += "</value></value>\n";
                }
                "redisinsight" => {
                    let h = host(p)?.trim_matches(['[', ']']).to_string();
                    let db = database(p);
                    let db = if db.is_empty() {
                        0
                    } else {
                        db.parse::<u32>()
                            .map_err(|_| invalid("Invalid Redis database index"))?
                    };
                    let mut value = json!({"id":id,"name":name,"host":h,"port":port(p,6379)?,"db":db,"username":field(p,"user"),"provider":"REDIS","connectionType":"STANDALONE","tls":tls_enabled(p),"verifyServerCert":tls_verify(p),"tlsServername":p["tls"]["server_name"],"nameFromProvider":null,"lastConnection":null,"modules":[],"compressor":"NONE"});
                    if include_passwords {
                        if let Some(password) = &profile.password {
                            value["password"] = json!(password);
                        }
                    }
                    for (key, target) in [("ca_cert", "caCert"), ("client_cert", "clientCert")] {
                        let certificate = field(&p["tls"], key);
                        if !certificate.is_empty() {
                            value[target] = json!({"id":format!("{id}-{key}"),"name":format!("{name} {key}"),"certificate":certificate});
                            if target == "clientCert" && include_passwords {
                                let key = field(&p["tls"], "client_key");
                                if !key.is_empty() {
                                    value[target]["key"] = json!(key);
                                }
                            }
                        }
                    }
                    native.push(value);
                }
                _ => unreachable!(),
            }
            result.contains_passwords |= include_passwords
                && if matches!(format, "json" | "csv") {
                    record != {
                        let mut sanitized = record.clone();
                        sanitize(&mut sanitized, false);
                        sanitized
                    }
                } else {
                    profile.password.is_some()
                        || (format == "redisinsight"
                            && !field(&record["params"]["tls"], "client_key").is_empty()
                            && !field(&record["params"]["tls"], "client_cert").is_empty())
                };
            records.push(record);
            Ok(())
        })();
        match operation {
            Ok(()) => result.exported_connections += 1,
            Err(error) => result.skipped.push(SkippedProfile {
                id,
                name,
                kind,
                reason: error.to_string(),
            }),
        }
    }
    let (name, mime, content) = match format {
        "json" => (
            "otto-connections.json",
            "application/json",
            stringify(&json!({"format":"otto-connections","version":1,"connections":records}))?,
        ),
        "csv" => ("otto-connections.csv", "text/csv;charset=utf-8", csv),
        "dbeaver_mysql" => ("dbeaver-mysql.csv", "text/csv;charset=utf-8", csv),
        "dbeaver_mongodb" => ("dbeaver-mongodb.csv", "text/csv;charset=utf-8", csv),
        "nosqlbooster" => (
            "nosqlbooster-mongodb.txt",
            "text/plain;charset=utf-8",
            uri_list,
        ),
        "redisinsight" => (
            "redisinsight-connections.json",
            "application/json",
            stringify(&native)?,
        ),
        "mysql_workbench" => {
            workbench += "</value></data>\n";
            ("connections.xml", "application/xml", workbench)
        }
        _ => unreachable!(),
    };
    result.files.push(ExportFile {
        name: name.into(),
        mime: mime.into(),
        content,
    });
    if format == "mysql_workbench" && result.contains_passwords {
        let credentials:Vec<_>=records.iter().filter(|r|r.get("password").is_some()).map(|r|json!({"id":r["id"],"name":r["name"],"host":r["params"]["host"],"user":r["params"]["user"],"password":r["password"]})).collect();
        result.files.push(ExportFile{name:"workbench-credentials.json".into(),mime:"application/json".into(),content:stringify(&json!({"instructions":"Enter these passwords manually into Workbench Store in Vault; this is not a native credential import.","connections":credentials}))?});
        result.warnings.push("Workbench cannot import portable vault passwords. The separate credential file contains plaintext passwords for manual entry.".into());
    }
    if result.contains_passwords {
        result.warnings.push("This prepared export contains plaintext credentials. Download only to a trusted location.".into());
    }
    if matches!(format, "json" | "csv") {
        result.warnings.push("Local paths and free-form commands are retained as configuration. Referenced SSH/private-key files are not read. Review custom command text before sharing.".into());
    }
    if result.files.iter().map(|f| f.content.len()).sum::<usize>() > 32 * 1024 * 1024 {
        return Err(invalid(
            "Connection export exceeds 32 MiB; select fewer workspaces",
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn profile(kind: &str) -> ExportProfile {
        ExportProfile {
            record: json!({"id":format!("id-{kind}"),"name":"Example \"quoted\", café","kind":kind,"workspace_id":"ws-a","params":{"host":"db.example.test","port":3306,"user":"alice","db":"app","advanced":{"retained":true}},"secret_ref":"opaque-key","first_command":null,"environment":"prod","read_only":true}),
            password: Some("p@ss:/%\"雪".into()),
        }
    }
    #[test]
    fn json_preserves_all_kinds_and_advanced_fields_but_default_never_leaks_credentials() {
        let kinds = [
            "ssh",
            "mysql",
            "redis",
            "mongodb",
            "clickhouse",
            "postgres",
            "custom",
        ];
        let mut profiles: Vec<_> = kinds.iter().map(|k| profile(k)).collect();
        profiles[3].record["params"]["conn_string"] = json!(
            "mongodb://a:legacy%40secret@one:27017,two:27018/db?authSource=admin&password=hidden"
        );
        profiles[0].record["params"]["nested"] =
            json!({"password":"nested-secret","token":"token-secret"});
        let result = render("json", profiles, false).unwrap();
        assert_eq!(result.exported_connections, 7);
        let text = &result.files[0].content;
        for secret in [
            "opaque-key",
            "legacy%40secret",
            "hidden",
            "nested-secret",
            "token-secret",
            "p@ss",
        ] {
            assert!(!text.contains(secret), "leaked {secret}");
        }
        let doc: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(doc["connections"].as_array().unwrap().len(), 7);
        assert_eq!(
            doc["connections"][0]["params"]["advanced"]["retained"],
            true
        );
        assert!(!result.contains_passwords);
    }
    #[test]
    fn explicit_json_and_csv_preserve_password_and_json_cells_losslessly() {
        let p = profile("custom");
        let json = render("json", vec![p.clone()], true).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&json.files[0].content).unwrap();
        assert_eq!(
            doc["connections"][0]["password"],
            p.password.as_deref().unwrap()
        );
        let csv = render("csv", vec![p.clone()], true).unwrap();
        let rows = parse_csv(&csv.files[0].content).unwrap();
        let column = rows[0].iter().position(|s| s == "record_json").unwrap();
        let record: serde_json::Value = serde_json::from_str(&rows[1][column]).unwrap();
        assert_eq!(record, doc["connections"][0]);
        assert!(csv.contains_passwords);
    }
    #[test]
    fn workbench_xml_is_parseable_and_password_goes_only_in_opt_in_sidecar() {
        let mut p = profile("mysql");
        p.record["params"]["tls"] =
            json!({"mode":"required","verify":true,"ca_file":"/tmp/a&b.pem"});
        let plain = render("mysql_workbench", vec![p.clone()], false).unwrap();
        assert_eq!(plain.files.len(), 1);
        let (parsed, warnings) = crate::conn_import::parse_mysql_workbench(&plain.files[0].content);
        assert!(warnings.is_empty());
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, p.record["name"].as_str().unwrap());
        assert_eq!(parsed[0].params["tls"]["verify"], true);
        let secret = render("mysql_workbench", vec![p.clone()], true).unwrap();
        assert_eq!(secret.files.len(), 2);
        assert!(!secret.files[0].content.contains("p@ss"));
        let sidecar: serde_json::Value = serde_json::from_str(&secret.files[1].content).unwrap();
        assert_eq!(sidecar["connections"][0]["password"], p.password.unwrap());
    }
    #[test]
    fn nosqlbooster_uri_keeps_replica_topology_and_encodes_password() {
        let mut p = profile("mongodb");
        p.record["params"]["conn_string"] =
            json!("mongodb://alice:{secret}@one:27017,two:27018/db?authSource=admin&tls=true");
        let secret = render("nosqlbooster", vec![p.clone()], true).unwrap();
        assert!(secret.files[0]
            .content
            .contains("alice:p%40ss%3A%2F%25%22%E9%9B%AA@one:27017,two:27018"));
        let plain = render("nosqlbooster", vec![p], false).unwrap();
        assert!(!plain.files[0].content.contains("{secret}"));
        assert!(plain.files[0]
            .content
            .contains("mongodb://alice@one:27017,two:27018/db?authSource=admin&tls=true"));
    }
    #[test]
    fn dbeaver_csv_round_trips_quotes_newlines_passwords_and_driver_specific_uri() {
        let mut p = profile("mysql");
        p.record["name"] = json!("quoted,\"name\"\nnext");
        let result = render("dbeaver_mysql", vec![p.clone()], true).unwrap();
        let rows = parse_csv(&result.files[0].content).unwrap();
        assert_eq!(rows[1][0], p.record["name"].as_str().unwrap());
        assert_eq!(rows[1][7], p.password.unwrap());
        assert!(rows[1][5].starts_with("jdbc:mysql://"));
    }
    #[test]
    fn redisinsight_native_json_carries_auth_tls_and_db_without_secrets_by_default() {
        let mut p = profile("redis");
        p.record["params"] = json!({"host":"cache.test","port":6380,"user":"acl","db":2,"tls":{"mode":"required","verify":true}});
        let plain = render("redisinsight", vec![p.clone()], false).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&plain.files[0].content).unwrap();
        assert_eq!(doc[0]["connectionType"], "STANDALONE");
        assert_eq!(doc[0]["db"], 2);
        assert_eq!(doc[0]["tls"], true);
        assert_eq!(doc[0]["username"], "acl");
        assert!(doc[0].get("password").is_none());
        let secret = render("redisinsight", vec![p.clone()], true).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&secret.files[0].content).unwrap();
        assert_eq!(doc[0]["password"], p.password.unwrap());
    }
    #[test]
    fn native_skips_incompatible_kinds_and_unsupported_transport_explicitly() {
        let mut mongo = profile("mongodb");
        mongo.record["params"]["jump"] = json!("bastion");
        let result = render("nosqlbooster", vec![profile("mysql"), mongo], true).unwrap();
        assert_eq!(result.total_connections, 2);
        assert_eq!(result.exported_connections, 0);
        assert_eq!(result.skipped.len(), 2);
        assert!(!result.contains_passwords);
        assert!(render("unknown", vec![], false).is_err());
    }
    #[test]
    fn native_database_alias_tls_shorthand_and_ipv6_preserve_effective_config() {
        let mut redis = profile("redis");
        redis.record["params"] = json!({"host":"::1","database":"4","secure":true});
        let result = render("redisinsight", vec![redis], false).unwrap();
        let rows: Value = serde_json::from_str(&result.files[0].content).unwrap();
        assert_eq!(rows[0]["host"], "::1");
        assert_eq!(rows[0]["port"], 6379);
        assert_eq!(rows[0]["db"], 4);
        assert_eq!(rows[0]["tls"], true);
        let mut mysql = profile("mysql");
        mysql.record["params"] = json!({"host":"::1","database":"schema one","secure":true});
        let result = render("dbeaver_mysql", vec![mysql], false).unwrap();
        let rows = parse_csv(&result.files[0].content).unwrap();
        assert_eq!(rows[1][4], "schema one");
        assert_eq!(
            rows[1][5],
            "jdbc:mysql://[::1]:3306/schema%20one?sslMode=VERIFY_IDENTITY"
        );
        let mut unsupported = profile("mysql");
        unsupported.record["params"]["ssl"] = json!(true);
        assert_eq!(
            render("dbeaver_mysql", vec![unsupported], false)
                .unwrap()
                .skipped
                .len(),
            1
        );
    }
    #[test]
    fn native_uri_list_uses_documented_comments_and_csv_never_changes_formula_like_passwords() {
        let mut p = profile("mongodb");
        p.record["params"]["conn_string"] = json!("mongodb+srv://alice:{secret}@cluster.test/app");
        assert!(render("nosqlbooster", vec![p], false).unwrap().files[0]
            .content
            .starts_with("// "));
        let mut p = profile("mysql");
        p.password = Some("=formula,\"line\"\nnext".into());
        let result = render("dbeaver_mysql", vec![p.clone()], true).unwrap();
        assert_eq!(
            parse_csv(&result.files[0].content).unwrap()[1][7],
            p.password.unwrap()
        );
    }
    #[test]
    fn mongo_uri_precedence_and_discrete_auth_options_match_runtime() {
        let mut p = profile("mongodb");
        p.record["params"] =
            json!({"conn_string":"mongodb://host.test/app?tls=false","secure":true});
        let output = render("nosqlbooster", vec![p.clone()], false).unwrap();
        assert!(output.files[0]
            .content
            .contains("mongodb://host.test/app?tls=false\n"));
        p.record["params"] = json!({"host":"mongo.test","database":"app","user":"alice","auth_source":"auth db","replica_set":"set one","secure":true});
        let output = render("nosqlbooster", vec![p], true).unwrap();
        let text = &output.files[0].content;
        assert!(text
            .contains("mongo.test:27017/app?tls=true&authSource=auth%20db&replicaSet=set%20one"));
    }
    #[test]
    fn opaque_markers_never_export_and_inline_private_keys_require_opt_in() {
        let mut p = profile("redis");
        p.record["params"]["db"] = json!(0);
        p.record["params"]["credentials"] = json!({"$secret":"opaque-marker-reference"});
        p.record["params"]["tls"] = json!({"mode":"required","ca_cert":"public certificate","client_cert":"public client cert","client_key":"private fixture key"});
        let default = render("json", vec![p.clone()], false).unwrap();
        assert!(default.files[0].content.contains("public certificate"));
        assert!(!default.files[0].content.contains("private fixture key"));
        let explicit = render("json", vec![p.clone()], true).unwrap();
        assert!(!explicit.files[0]
            .content
            .contains("opaque-marker-reference"));
        let native = render("redisinsight", vec![p], true).unwrap();
        assert_eq!(native.exported_connections, 1);
        let doc: Value = serde_json::from_str(&native.files[0].content).unwrap();
        assert_eq!(doc[0]["clientCert"]["key"], "private fixture key");
    }
}
