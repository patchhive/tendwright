use axum::{routing::MethodRouter, Router};

pub const STANDARD_SPECIALIST_ROUTE_PATHS: &[&str] = &[
    "/auth/status",
    "/auth/login",
    "/auth/generate-key",
    "/auth/generate-service-token",
    "/auth/rotate-service-token",
    "/health",
    "/startup/checks",
    "/capabilities",
    "/runs",
    "/runs/:id",
    "/overview",
    "/history",
    "/history/:id",
];

pub struct SpecialistRouteHandlers<S> {
    pub auth_status: MethodRouter<S>,
    pub login: MethodRouter<S>,
    pub generate_key: MethodRouter<S>,
    pub generate_service_token: MethodRouter<S>,
    pub rotate_service_token: MethodRouter<S>,
    pub health: MethodRouter<S>,
    pub startup_checks: MethodRouter<S>,
    pub capabilities: MethodRouter<S>,
    pub runs: MethodRouter<S>,
    pub run_detail: MethodRouter<S>,
    pub overview: MethodRouter<S>,
    pub history: MethodRouter<S>,
    pub history_detail: MethodRouter<S>,
}

pub fn standard_specialist_router<S>(handlers: SpecialistRouteHandlers<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let methods = [
        handlers.auth_status,
        handlers.login,
        handlers.generate_key,
        handlers.generate_service_token,
        handlers.rotate_service_token,
        handlers.health,
        handlers.startup_checks,
        handlers.capabilities,
        handlers.runs,
        handlers.run_detail,
        handlers.overview,
        handlers.history,
        handlers.history_detail,
    ];

    STANDARD_SPECIALIST_ROUTE_PATHS
        .iter()
        .copied()
        .zip(methods)
        .fold(Router::new(), |router, (path, method)| {
            router.route(path, method)
        })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::STANDARD_SPECIALIST_ROUTE_PATHS;

    #[test]
    fn standard_route_paths_are_absolute_and_unique() {
        assert!(STANDARD_SPECIALIST_ROUTE_PATHS
            .iter()
            .all(|path| path.starts_with('/')));
        assert_eq!(
            STANDARD_SPECIALIST_ROUTE_PATHS.len(),
            STANDARD_SPECIALIST_ROUTE_PATHS
                .iter()
                .collect::<HashSet<_>>()
                .len()
        );
    }
}
