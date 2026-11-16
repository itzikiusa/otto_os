use super::*;
use axum::http::Method;

#[test]
fn ide_routes_require_agents_edit_except_guest_collab_and_person_only_admin() {
    use Capability::{Edit, View};
    use Feature::{Agents, Git};
    use PolicyDecision::{Exempt, Require};

    let pol = |m: Method, p: &str| policy_for(&m, p);
    assert_eq!(
        pol(Method::GET, "/api/v1/ide/fs/tree"),
        Require(Agents, Edit)
    );
    assert_eq!(
        pol(Method::GET, "/api/v1/ide/fs/git-base"),
        Require(Agents, Edit)
    );
    assert_eq!(
        pol(Method::GET, "/api/v1/repos/{id}/file"),
        Require(Git, View)
    );
    assert_eq!(
        pol(Method::GET, "/api/v1/ide/toolchains"),
        Require(Agents, Edit)
    );
    assert_eq!(
        pol(Method::GET, "/api/v1/ide/collab/guest/{room_id}/state"),
        Exempt
    );
    assert_eq!(
        pol(Method::POST, "/api/v1/ide/collab/guest/{room_id}/edit"),
        Exempt
    );
    assert_eq!(
        route_class(&Method::POST, "/api/v1/ide/collab/rooms"),
        Some(RouteClass::Secret)
    );
    assert_eq!(
        route_class(&Method::POST, "/api/v1/ide/toolchains/install"),
        Some(RouteClass::Admin)
    );
    assert_eq!(
        route_class(&Method::POST, "/api/v1/workspaces/{id}/lsp/install"),
        Some(RouteClass::Admin)
    );
    assert_eq!(
        route_class(&Method::GET, "/api/v1/workspaces/{id}/lsp/install"),
        None
    );
    assert_eq!(
        route_class(&Method::POST, "/api/v1/ide/plugins/install"),
        Some(RouteClass::Admin)
    );
    assert_eq!(
        route_class(&Method::POST, "/api/v1/ide/plugins/{slug}/toggle"),
        Some(RouteClass::Admin)
    );
    assert_eq!(
        route_class(&Method::DELETE, "/api/v1/ide/plugins/{slug}"),
        Some(RouteClass::Admin)
    );
    assert_eq!(route_class(&Method::GET, "/api/v1/ide/plugins"), None);
    assert_eq!(
        pol(Method::GET, "/api/v1/ide/lsp/status"),
        Require(Agents, Edit)
    );
    assert_eq!(
        pol(Method::GET, "/api/v1/ide/runs/{run_id}/events"),
        Require(Agents, Edit)
    );
    assert_eq!(
        route_class(&Method::POST, "/api/v1/ide/build-tools/model"),
        None
    );
    assert_eq!(
        route_class(&Method::POST, "/api/v1/ide/run-configs"),
        Some(RouteClass::Admin)
    );
    assert_eq!(
        route_class(&Method::PUT, "/api/v1/ide/run-configs/{id}"),
        Some(RouteClass::Admin)
    );
    assert_eq!(route_class(&Method::GET, "/api/v1/ide/run-configs"), None);
    assert_eq!(
        route_class(&Method::POST, "/api/v1/ide/debug/sessions/{id}/evaluate"),
        Some(RouteClass::Admin)
    );
    assert_eq!(
        route_class(&Method::POST, "/api/v1/ide/debug/sessions/{id}/command"),
        Some(RouteClass::Admin)
    );
    assert_eq!(
        route_class(&Method::GET, "/api/v1/ide/debug/sessions/{id}"),
        None
    );
}
