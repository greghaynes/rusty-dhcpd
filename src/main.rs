mod config;
mod dhcpd;
mod web;

#[macro_use]
extern crate slog;
extern crate slog_async;
extern crate slog_term;

use slog::Drain;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;
use warp::Filter;

const SERVER_IP: Ipv4Addr = Ipv4Addr::new(10, 60, 0, 1);
const SERVER_PORT: u16 = 9067;

fn init_config() -> config::Config {
    return config::Config {
        server_ip: SERVER_IP,
        listen_port: SERVER_PORT,
        bind_interface: None,
        lease_start: Ipv4Addr::new(10, 60, 0, 10),
        lease_count: 24,
        lease_duration: Duration::from_secs(3),
        lease_subnet_mask: Ipv4Addr::new(255, 255, 255, 0),
        lease_routers: vec![Ipv4Addr::new(10, 60, 0, 1)],
        lease_domain_servers: vec![Ipv4Addr::new(8, 8, 8, 8)],
    };
}

fn init_logging() -> slog::Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    return slog::Logger::root(drain, o!());
}

#[tokio::main]
async fn main() {
    let logger = init_logging();
    let config = init_config();

    let dhcp_shutdown = Arc::new(tokio::sync::Notify::new());

    let dhcp_srv = Arc::new(dhcpd::server::Server::create(&config, logger));
    let dhcp_srv_web = dhcp_srv.clone();

    let dhcp_srv_filter =
        warp::any().map(move || -> Arc<dyn dhcpd::server::AbstractServer> { dhcp_srv_web.clone() });
    let leases = warp::path("leases")
        .and(dhcp_srv_filter)
        .and_then(web::handlers::leases_handler);
    let routes = leases;

    tokio::select! {
        _ = dhcp_srv.serve(dhcp_shutdown) => {},
        _ = warp::serve(routes).run(([0, 0, 0, 0], 3030)) => {}
    }
}
