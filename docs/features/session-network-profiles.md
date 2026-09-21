# Session network profiles

Use a network profile when a program run by an agent or shell needs a database/service reachable from an SSH bastion. Otto opens localhost TCP forwards for that session and supplies explicit host/port variables. The program uses those addresses just as it would use any other TCP endpoint. This does not change your machine's VPN, routes or global proxy settings.

## Setup and walkthrough

1. Save an **SSH connection** in Connections for the bastion. The system SSH client uses its host, port, optional username and identity file, along with your existing ssh-agent, SSH configuration and known hosts. The bastion must permit TCP forwarding. This feature uses unattended SSH authentication; it does not ask for passwords inside an agent terminal.
2. In **New session**, open the network-profile controls and create a profile. Give it a name and select the SSH connection.
3. Add one or more endpoints. For a PostgreSQL service, for example, use name `database`, remote host `db.office.internal`, and remote port `5432`. These are resolved/reached from the bastion's network, not your laptop's DNS/network.
4. Optionally map the endpoint to environment names your application already reads. For PostgreSQL tools choose host variable `PGHOST` and port variable `PGPORT`; an application may instead use `DB_HOST` and `DB_PORT`.
5. Save, select the profile and launch your chosen agent or shell. Otto prepares every forward before starting the session. An authorization/SSH error fails that launch; it does not launch a direct-network replacement.
6. Expand the session's network status to see each actual localhost endpoint, remote destination and environment mapping. **Connected** means the SSH forwards are listening; a database can still reject credentials or be unreachable from the bastion.

Each profile supports 1–8 endpoints. Endpoint names begin with an ASCII letter and contain letters, digits or underscores, up to 32 characters; names are unique ignoring case. Remote hosts accept DNS/IPv4 or bracketed IPv6. Explicit variable names must end in `_HOST` / `_PORT` or be `PGHOST` / `PGPORT`, use uppercase letters/digits/underscores, and cannot use reserved `OTTO_`, `LD_` or `DYLD_` prefixes. Duplicate environment mappings are rejected.

The profile contains addresses and variable names only. Database usernames/passwords remain in your program's existing credential configuration. SSH keys remain in the normal SSH configuration; no key/password content is copied into a profile.

## What a program receives

For endpoint `database`, Otto always injects:

```text
OTTO_TUNNEL_DATABASE_HOST=127.0.0.1
OTTO_TUNNEL_DATABASE_PORT=<ephemeral local port>
```

If you chose `PGHOST` and `PGPORT`, those point to the same localhost endpoint, so `psql` can use them with its normal database/user/password settings. Otto also supplies `OTTO_NETWORK_ENDPOINTS`, a JSON array with endpoint name, local host/port, remote host/port and optional mapped variable names.

A generic Go TCP client can explicitly consume a mapped endpoint:

```go
addr := net.JoinHostPort(os.Getenv("DB_HOST"), os.Getenv("DB_PORT"))
conn, err := net.DialTimeout("tcp", addr, 5*time.Second)
if err != nil {
    return err
}
defer conn.Close()
```

Configure your actual database driver with those values. Generic Go socket calls do not automatically use `HTTP_PROXY`; this feature does not set it. Hardcoded remote hosts in application code remain hardcoded until you configure the program to use the forwarded address.

## Lifetime, edits and status

Forwards belong to one live session. Terminating, suspending, archiving, removing or replacing the session closes its forwards. Natural PTY exit also closes them. If startup fails partway through several endpoints, all forwards opened by that attempt are released. A replacement tunnel is prepared before an existing session is stopped, so a tunnel-preparation failure leaves that running session intact.

A broken SSH child changes the status to **Error** and preserves its diagnostic. Restart the session after fixing connectivity. Otto does not silently reopen at a new port while a running program still has the old port in its environment. The OS may take the SSH keepalive interval to detect a broken network.

Changing a session's selected profile or editing the saved profile applies at the next restart. The status view flags the mismatch between selected configuration and the active snapshot. Profile edits use a version check; if someone saved a newer version, reload it before retrying your save. Archiving a profile preserves its identity/history but prevents new launches with it.

Graceful daemon shutdown closes managed forwards. After an abrupt process crash such as SIGKILL, normal destructor cleanup cannot run; this implementation does not guarantee termination of already orphaned SSH children. A subsequent session restart establishes a fresh set of mappings.

## Permissions and boundaries

Profiles are workspace-scoped. Viewing requires the workspace and Connections feature access; saving requires Editor plus Connections Edit. Using a bastion also requires its existing resource `shell` access (including discovery under an enforced policy). These permissions are checked again at session launch/restart and when reading network status. An ordinary project or profile selection grants no additional connection permission. Status is available only to the session owner or workspace administrator and still checks effective bastion access.

SSH forwarding carries raw TCP. Otto's Database Explorer query/write approvals do not inspect SQL sent by an independently run application. Configure that application's native database credentials for the access it should have. Session sandbox network restrictions remain unchanged.

TLS verification stays enabled according to your application's configuration. When the TCP destination becomes localhost, a TLS client may need a separate server-name/SNI setting for the original remote host and its normal CA trust. This feature does not bypass certificate checks.

MongoDB replica-set discovery and Kafka advertised broker addresses may point at additional remote hosts after the initial connection; one local forward does not rewrite those addresses. MongoDB `directConnection=true` can target a single server where that mode fits the deployment; use the appropriate TLS server-name configuration. Replica-set failover, SRV discovery and Kafka topology-aware routing are separate from these generic TCP forwards.

## API

See the [API contract](../contracts/api.md#session-network-profiles) for payloads and authorization.

| Method and path (under `/api/v1`) | Purpose |
| --- | --- |
| `GET /workspaces/{id}/network-profiles` | List profiles the caller can use, including archived profiles. |
| `POST /workspaces/{id}/network-profiles` | Create a profile after validating its SSH connection. |
| `GET /network-profiles/{id}` | Read a profile. |
| `PUT /network-profiles/{id}` | Replace editable configuration with the expected `version`. |
| `GET /sessions/{id}/network` | Read actual forwarding status, endpoints and restart requirement. |

Session create/update metadata uses `network_profile_id` (ID or null). Updates change the next-launch configuration and do not mutate the environment of an already running program. Network status uses polling; there is no new WebSocket event.

## Troubleshooting

| Symptom | What to inspect |
| --- | --- |
| No profiles appear | Confirm workspace, Connections permissions, and bastion discovery/shell permission. |
| Launch fails with SSH authentication error | Check the existing SSH connection, ssh-agent and known hosts. No interactive password prompt is supported. |
| Connection has a `jump` parameter | This forwarding helper uses `ProxyJump` in `~/.ssh/config`; configure the jump there. Unsupported saved `jump` parameters fail explicitly. |
| SSH is connected but DB connection fails | Check remote host/port from the bastion, forwarding policy, database service and application credentials. |
| App still dials the office hostname | Configure it to read the mapped host/port or the displayed localhost endpoint. |
| TLS hostname mismatch | Configure the client’s expected remote TLS name while retaining its CA validation. |
| Port changed after restart | Expected: forwards allocate free localhost ports per session launch. Read the new environment or status. |
| Session shows Error | Read the preserved SSH diagnostic, fix connectivity and restart. |
| Save reports a conflict | Keep your unsaved edits, reload the latest profile, reconcile and save its current version. |

Implementation: core contract `network_profiles.rs`; state migration `0137` and `NetworkProfilesRepo`; session lifecycle/runtime in `otto-sessions/src/network.rs`; routes in `otto-server/src/routes/network_profiles.rs`; profile and status controls in `ui/src/modules/connections/`.
