//! Parser unit tests over SYNTHETIC fixtures only — never the user's real
//! config files. Fixtures mirror the real on-disk shapes (verified read-only
//! against the author's machine, with all hosts/users/secrets fabricated).

use super::*;
use serde_json::json;

// --- parse_jdbc_url ---------------------------------------------------------

#[test]
fn jdbc_mysql_full() {
    let j = parse_jdbc_url("jdbc:mysql://db.example.com:3306/appdb?useSSL=true").unwrap();
    assert_eq!(j.scheme, "mysql");
    assert_eq!(j.host, "db.example.com");
    assert_eq!(j.port, Some(3306));
    assert_eq!(j.db.as_deref(), Some("appdb"));
}

#[test]
fn jdbc_clickhouse_and_no_db() {
    let j = parse_jdbc_url("jdbc:clickhouse://ch.example.com:8123").unwrap();
    assert_eq!(j.scheme, "clickhouse");
    assert_eq!(j.host, "ch.example.com");
    assert_eq!(j.port, Some(8123));
    assert_eq!(j.db, None);
}

#[test]
fn jdbc_no_port() {
    let j = parse_jdbc_url("jdbc:mysql://localhost/mydb").unwrap();
    assert_eq!(j.host, "localhost");
    assert_eq!(j.port, None);
    assert_eq!(j.db.as_deref(), Some("mydb"));
}

#[test]
fn jdbc_userinfo_stripped_and_replica_set() {
    // userinfo dropped from host; first host of a replica list used.
    let j = parse_jdbc_url("jdbc:mongodb://user:pw@a.example.com:27017,b.example.com:27017/admin")
        .unwrap();
    assert_eq!(j.host, "a.example.com");
    assert_eq!(j.port, Some(27017));
    assert_eq!(j.db.as_deref(), Some("admin"));
}

#[test]
fn jdbc_ipv6() {
    let j = parse_jdbc_url("jdbc:mysql://[::1]:3306/db").unwrap();
    assert_eq!(j.host, "::1");
    assert_eq!(j.port, Some(3306));
}

#[test]
fn jdbc_garbage_is_none() {
    assert!(parse_jdbc_url("not a url").is_none());
    assert!(parse_jdbc_url("jdbc:mysql://").is_none());
}

// --- kind_for_engine --------------------------------------------------------

#[test]
fn engine_mapping() {
    assert_eq!(kind_for_engine("mysql").unwrap(), ConnectionKind::Mysql);
    assert_eq!(kind_for_engine("mysql8").unwrap(), ConnectionKind::Mysql);
    assert_eq!(kind_for_engine("mariadb").unwrap(), ConnectionKind::Mysql);
    assert_eq!(
        kind_for_engine("com.mysql.rdbms.mysql.driver.native").unwrap(),
        ConnectionKind::Mysql
    );
    assert_eq!(
        kind_for_engine("com_clickhouse").unwrap(),
        ConnectionKind::Clickhouse
    );
    assert_eq!(kind_for_engine("mongo").unwrap(), ConnectionKind::Mongodb);
    assert_eq!(kind_for_engine("redis").unwrap(), ConnectionKind::Redis);
    // Unsupported engines carry an explanatory note.
    assert!(kind_for_engine("postgresql")
        .unwrap_err()
        .contains("PostgreSQL"));
    assert!(kind_for_engine("oracle").unwrap_err().contains("Oracle"));
    assert!(kind_for_engine("mssql").unwrap_err().contains("SQL Server"));
    assert!(kind_for_engine("sqlite").unwrap_err().contains("SQLite"));
}

// --- MySQL Workbench --------------------------------------------------------

const WB_XML: &str = r#"<?xml version="1.0"?>
<data grt_format="2.0">
 <value type="list" content-type="object" content-struct-name="db.mgmt.Connection">
  <value type="object" struct-name="db.mgmt.Connection" id="AAA">
   <link type="object" struct-name="db.mgmt.Driver" key="driver">com.mysql.rdbms.mysql.driver.native</link>
   <value type="dict" key="parameterValues">
    <value type="string" key="hostName">db.example.com</value>
    <value type="int" key="port">3306</value>
    <value type="string" key="userName">appuser</value>
    <value type="string" key="schema">appdb</value>
    <value type="int" key="useSSL">0</value>
   </value>
   <value type="string" key="name">Local App DB</value>
  </value>
  <value type="object" struct-name="db.mgmt.Connection" id="BBB">
   <link type="object" struct-name="db.mgmt.Driver" key="driver">com.mysql.rdbms.mysql.driver.native</link>
   <value type="dict" key="parameterValues">
    <value type="string" key="hostName">secure.example.com</value>
    <value type="int" key="port">3307</value>
    <value type="string" key="userName">admin</value>
    <value type="string" key="schema"></value>
    <value type="int" key="useSSL">2</value>
    <value type="string" key="sslCA">/etc/ssl/ca.pem</value>
   </value>
   <value type="string" key="name">Secure DB (SSL)</value>
  </value>
 </value>
</data>"#;

#[test]
fn workbench_normal_and_ssl() {
    let (conns, warnings) = parse_mysql_workbench(WB_XML);
    assert!(warnings.is_empty(), "warnings: {warnings:?}");
    assert_eq!(conns.len(), 2);

    let a = &conns[0];
    assert_eq!(a.name, "Local App DB");
    assert_eq!(a.kind, Some(ConnectionKind::Mysql));
    assert!(a.supported);
    assert!(a.needs_password);
    assert_eq!(
        a.params,
        json!({"host":"db.example.com","port":3306,"user":"appuser","db":"appdb"})
    );

    let b = &conns[1];
    assert_eq!(b.name, "Secure DB (SSL)");
    // Empty schema → no db; useSSL=2 → "Require" mode but NOT verify (level 2 only
    // encrypts; verify is levels 3/4). mode is the valid `required` (not `require`)
    // and verify is emitted explicitly as false so the import isn't forced to
    // validate a self-signed cert. The CA is still carried through.
    assert_eq!(
        b.params,
        json!({
            "host":"secure.example.com","port":3307,"user":"admin",
            "tls":{"mode":"required","verify":false,"ca_cert":"/etc/ssl/ca.pem"}
        })
    );
}

#[test]
fn workbench_ssl_levels_map_to_mode_and_verify() {
    // useSSL 1=preferred/no-verify, 2=required/no-verify, 3=required/verify,
    // 4=required/verify. One <db.mgmt.Connection> per level.
    let mk = |level: i64| {
        format!(
            r#"<?xml version="1.0"?>
<data><value type="list">
 <value type="object" struct-name="db.mgmt.Connection" id="X">
  <value type="dict" key="parameterValues">
   <value type="string" key="hostName">h</value>
   <value type="int" key="port">3306</value>
   <value type="int" key="useSSL">{level}</value>
  </value>
  <value type="string" key="name">c</value>
 </value>
</value></data>"#
        )
    };
    let cases = [
        (1, "preferred", false),
        (2, "required", false),
        (3, "required", true),
        (4, "required", true),
    ];
    for (level, mode, verify) in cases {
        let (conns, _) = parse_mysql_workbench(&mk(level));
        let tls = &conns[0].params["tls"];
        // mode must be a valid TlsMode spelling (the bug emitted "require"); the
        // round-trip through the real enum is asserted in otto-dbviewer's tests.
        assert_eq!(tls["mode"], json!(mode), "useSSL={level}");
        assert_eq!(tls["verify"], json!(verify), "useSSL={level}");
    }
}

#[test]
fn workbench_missing_host_skipped() {
    let xml = r#"<?xml version="1.0"?><data>
     <value type="object" struct-name="db.mgmt.Connection">
      <value type="dict" key="parameterValues">
       <value type="string" key="userName">x</value>
      </value>
      <value type="string" key="name">No Host</value>
     </value></data>"#;
    let (conns, warnings) = parse_mysql_workbench(xml);
    assert!(conns.is_empty());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("No Host"));
}

// --- DBeaver ----------------------------------------------------------------

const DBEAVER_JSON: &str = r#"{
  "folders": {},
  "connections": {
    "mysql-1": {
      "provider": "mysql",
      "driver": "mysql8",
      "name": "Prod MySQL",
      "save-password": true,
      "configuration": {
        "host": "mysql.example.com",
        "port": "3306",
        "url": "jdbc:mysql://mysql.example.com:3306",
        "auth-model": "native",
        "handlers": {
          "ssh_tunnel": {
            "type": "tunnel",
            "enabled": true,
            "properties": {
              "host": "bastion.example.com",
              "port": "22",
              "authType": "PUBLIC_KEY",
              "keyPath": "/home/me/.ssh/id_rsa",
              "userName": "deploy"
            }
          }
        }
      }
    },
    "ch-1": {
      "provider": "clickhouse",
      "driver": "com_clickhouse",
      "name": "Analytics CH",
      "configuration": {
        "host": "ch.example.com",
        "port": "8443",
        "database": "metrics",
        "url": "jdbc:clickhouse://ch.example.com:8443/metrics",
        "handlers": { "clickhouse-ssl": { "enabled": true } }
      }
    },
    "pg-1": {
      "provider": "postgresql",
      "driver": "postgres-jdbc",
      "name": "Reporting PG",
      "configuration": {
        "host": "pg.example.com", "port": "5432",
        "url": "jdbc:postgresql://pg.example.com:5432/reports"
      }
    }
  }
}"#;

#[test]
fn dbeaver_mysql_with_ssh_tunnel() {
    let (conns, warnings) = parse_dbeaver(DBEAVER_JSON);
    assert!(warnings.is_empty(), "warnings: {warnings:?}");
    let mysql = conns.iter().find(|c| c.name == "Prod MySQL").unwrap();
    assert_eq!(mysql.kind, Some(ConnectionKind::Mysql));
    assert!(mysql.supported);
    assert_eq!(mysql.params["host"], json!("mysql.example.com"));
    assert_eq!(mysql.params["port"], json!(3306));
    // Nested SSH tunnel block.
    assert_eq!(mysql.params["ssh"]["host"], json!("bastion.example.com"));
    assert_eq!(mysql.params["ssh"]["port"], json!(22));
    assert_eq!(mysql.params["ssh"]["user"], json!("deploy"));
    assert_eq!(
        mysql.params["ssh"]["identity_file"],
        json!("/home/me/.ssh/id_rsa")
    );
}

#[test]
fn dbeaver_clickhouse_with_ssl() {
    let (conns, _) = parse_dbeaver(DBEAVER_JSON);
    let ch = conns.iter().find(|c| c.name == "Analytics CH").unwrap();
    assert_eq!(ch.kind, Some(ConnectionKind::Clickhouse));
    assert_eq!(ch.params["host"], json!("ch.example.com"));
    assert_eq!(ch.params["port"], json!(8443));
    assert_eq!(ch.params["db"], json!("metrics"));
    assert_eq!(ch.params["tls"], json!({"mode":"required","verify":false}));
}

#[test]
fn dbeaver_postgres_unsupported() {
    let (conns, _) = parse_dbeaver(DBEAVER_JSON);
    let pg = conns.iter().find(|c| c.name == "Reporting PG").unwrap();
    assert_eq!(pg.kind, None);
    assert!(!pg.supported);
    assert!(pg.note.as_ref().unwrap().contains("PostgreSQL"));
}

#[test]
fn dbeaver_ssh_disabled_not_emitted() {
    let j = r#"{"connections":{"c":{"provider":"mysql","name":"X",
      "configuration":{"host":"h","port":"3306",
        "handlers":{"ssh_tunnel":{"enabled":false,"properties":{"host":"b","port":"22"}}}}}}}"#;
    let (conns, _) = parse_dbeaver(j);
    assert!(conns[0].params.get("ssh").is_none());
}

// --- DataGrip ---------------------------------------------------------------

const DG_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="DataSourceManagerImpl" format="xml">
    <data-source source="LOCAL" name="MySQL Local" uuid="uuid-mysql">
      <driver-ref>mysql.8</driver-ref>
      <jdbc-url>jdbc:mysql://localhost:3306/shop</jdbc-url>
    </data-source>
    <data-source source="LOCAL" name="CH Cluster" uuid="uuid-ch">
      <driver-ref>clickhouse</driver-ref>
      <jdbc-url>jdbc:clickhouse://ch.internal:9000</jdbc-url>
    </data-source>
    <data-source source="LOCAL" name="Mongo" uuid="uuid-mongo">
      <driver-ref>mongo</driver-ref>
      <jdbc-url>jdbc:mongodb://mongo.example.com:27017/store</jdbc-url>
    </data-source>
    <data-source source="LOCAL" name="Postgres" uuid="uuid-pg">
      <driver-ref>postgresql</driver-ref>
      <jdbc-url>jdbc:postgresql://pg.example.com:5432/rep</jdbc-url>
    </data-source>
  </component>
</project>"#;

const DG_LOCAL_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="dataSourceStorageLocal" created-in="DataGrip">
    <data-source name="MySQL Local" uuid="uuid-mysql">
      <database-info product="MySQL"/>
      <user-name>shopadmin</user-name>
      <ssl-config><mode>require</mode></ssl-config>
    </data-source>
    <data-source name="Mongo" uuid="uuid-mongo">
      <user-name>mongouser</user-name>
    </data-source>
  </component>
</project>"#;

#[test]
fn datagrip_local_parses_user_and_ssl() {
    let local = parse_datagrip_local(DG_LOCAL_XML);
    assert_eq!(local.users.get("uuid-mysql").unwrap(), "shopadmin");
    assert_eq!(local.users.get("uuid-mongo").unwrap(), "mongouser");
    assert_eq!(local.ssl.get("uuid-mysql"), Some(&true));
}

#[test]
fn datagrip_mysql_joins_local_user_and_ssl() {
    let local = parse_datagrip_local(DG_LOCAL_XML);
    let (conns, warnings) = parse_datagrip(DG_XML, &local);
    assert!(warnings.is_empty(), "warnings: {warnings:?}");
    let my = conns.iter().find(|c| c.name == "MySQL Local").unwrap();
    assert_eq!(my.kind, Some(ConnectionKind::Mysql));
    assert_eq!(my.params["host"], json!("localhost"));
    assert_eq!(my.params["port"], json!(3306));
    assert_eq!(my.params["db"], json!("shop"));
    assert_eq!(my.params["user"], json!("shopadmin"));
    assert_eq!(my.params["tls"], json!({"mode":"required","verify":false}));
    assert!(my.needs_password);
}

#[test]
fn datagrip_clickhouse_no_db_no_user() {
    let local = parse_datagrip_local(DG_LOCAL_XML);
    let (conns, _) = parse_datagrip(DG_XML, &local);
    let ch = conns.iter().find(|c| c.name == "CH Cluster").unwrap();
    assert_eq!(ch.kind, Some(ConnectionKind::Clickhouse));
    assert_eq!(ch.params["host"], json!("ch.internal"));
    assert_eq!(ch.params["port"], json!(9000));
    assert!(ch.params.get("db").is_none());
    assert!(ch.params.get("user").is_none());
    assert!(ch.params.get("tls").is_none());
    assert!(!ch.needs_password);
}

#[test]
fn datagrip_mongo_builds_conn_string() {
    let local = parse_datagrip_local(DG_LOCAL_XML);
    let (conns, _) = parse_datagrip(DG_XML, &local);
    let mongo = conns.iter().find(|c| c.name == "Mongo").unwrap();
    assert_eq!(mongo.kind, Some(ConnectionKind::Mongodb));
    // user known → {secret} placeholder present.
    assert_eq!(
        mongo.params["conn_string"],
        json!("mongodb://mongouser:{secret}@mongo.example.com:27017/store")
    );
    assert!(mongo.needs_password);
}

#[test]
fn datagrip_postgres_unsupported() {
    let local = parse_datagrip_local(DG_LOCAL_XML);
    let (conns, _) = parse_datagrip(DG_XML, &local);
    let pg = conns.iter().find(|c| c.name == "Postgres").unwrap();
    assert!(!pg.supported);
    assert_eq!(pg.kind, None);
    assert!(pg.note.as_ref().unwrap().contains("PostgreSQL"));
}

// --- NoSQLBooster -----------------------------------------------------------

const NSB_JSON: &str = r#"{
  "connections": [
    {
      "name": "Atlas SRV",
      "uri": {
        "scheme": "mongodb+srv",
        "hosts": [ { "host": "cluster0.abcde.mongodb.net" } ],
        "database": "prod",
        "username": "svcuser",
        "options": { "authSource": "admin", "ssl": true }
      }
    },
    {
      "name": "Replica Set",
      "uri": {
        "scheme": "mongodb",
        "hosts": [
          { "host": "m1.example.com", "port": 27017, "role": "primary" },
          { "host": "m2.example.com", "port": 27017, "role": "secondary" },
          { "host": "m3.example.com", "port": 27017, "role": "secondary" }
        ],
        "username": "rsadmin",
        "options": { "authSource": "admin", "replicaSet": "rs0" }
      }
    },
    {
      "name": "Local No Auth",
      "uri": {
        "scheme": "mongodb",
        "hosts": [ { "host": "127.0.0.1", "port": 27017 } ],
        "database": "test",
        "options": {}
      }
    }
  ]
}"#;

#[test]
fn nosqlbooster_srv() {
    let (conns, warnings) = parse_nosqlbooster(NSB_JSON);
    assert!(warnings.is_empty(), "warnings: {warnings:?}");
    let srv = conns.iter().find(|c| c.name == "Atlas SRV").unwrap();
    assert_eq!(srv.kind, Some(ConnectionKind::Mongodb));
    assert!(srv.needs_password);
    let cs = srv.params["conn_string"].as_str().unwrap();
    // srv → no ports; username → {secret} placeholder; options preserved.
    assert!(cs.starts_with("mongodb+srv://svcuser:{secret}@cluster0.abcde.mongodb.net/prod?"));
    assert!(cs.contains("authSource=admin"));
    assert!(cs.contains("ssl=true"));
    assert!(!cs.contains(":27017"));
}

#[test]
fn nosqlbooster_replica_set() {
    let (conns, _) = parse_nosqlbooster(NSB_JSON);
    let rs = conns.iter().find(|c| c.name == "Replica Set").unwrap();
    let cs = rs.params["conn_string"].as_str().unwrap();
    // All three hosts with ports, comma-joined; no database (empty path).
    assert!(cs.starts_with(
        "mongodb://rsadmin:{secret}@m1.example.com:27017,m2.example.com:27017,m3.example.com:27017/?"
    ));
    assert!(cs.contains("replicaSet=rs0"));
    assert!(cs.contains("authSource=admin"));
}

#[test]
fn nosqlbooster_no_auth_omits_secret() {
    let (conns, _) = parse_nosqlbooster(NSB_JSON);
    let local = conns.iter().find(|c| c.name == "Local No Auth").unwrap();
    assert!(!local.needs_password);
    assert_eq!(
        local.params["conn_string"],
        json!("mongodb://127.0.0.1:27017/test")
    );
}

// --- json_port leniency -----------------------------------------------------

#[test]
fn json_port_accepts_number_string_and_float_string() {
    assert_eq!(json_port(Some(&json!(3306))), Some(3306));
    assert_eq!(json_port(Some(&json!("3306"))), Some(3306));
    // DBeaver writes ssh ports as `22.0` sometimes.
    assert_eq!(json_port(Some(&json!(22.0))), Some(22));
    assert_eq!(json_port(Some(&json!("22.0"))), Some(22));
    assert_eq!(json_port(Some(&json!("not-a-port"))), None);
    assert_eq!(json_port(None), None);
}

// --- dedupe -----------------------------------------------------------------

#[test]
fn dedupe_drops_identical() {
    let mk = || ParsedConnection {
        source: ImportSource::Dbeaver,
        name: "Dup".into(),
        kind: Some(ConnectionKind::Mysql),
        params: json!({"host":"h","port":3306}),
        supported: true,
        needs_password: true,
        note: None,
    };
    let out = dedupe(vec![mk(), mk()]);
    assert_eq!(out.len(), 1);
}
