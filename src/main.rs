mod config;
mod leases;
mod server;

#[macro_use]
extern crate slog;
extern crate slog_async;
extern crate slog_term;

use dhcp4r::server as dhcp4rserver;
use slog::Drain;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;

const SERVER_IP: Ipv4Addr = Ipv4Addr::new(0, 0, 0, 0);
const SERVER_PORT: u16 = 67;

#[tokio::main]
async fn main() {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let logger = slog::Logger::root(drain, o!());
    let config = config::Config {
        bind_address: SocketAddrV4::new(SERVER_IP, SERVER_PORT),
        bind_interface: Some("dhcp-server-if".to_string()),
        lease_start: Ipv4Addr::new(10, 41, 0, 0),
        lease_count: 24,
        lease_duration: Duration::from_secs(300),
        lease_subnet_mask: Ipv4Addr::new(255, 255, 255, 0),
        lease_routers: vec![Ipv4Addr::new(10, 41, 1, 1)],
        lease_domain_servers: vec![Ipv4Addr::new(8, 8, 8, 8)],
    };

    let mut srv = server::Server::create(&config, logger);
    let shutdown_notify = std::sync::Arc::new(tokio::sync::Notify::new());
    srv.serve(shutdown_notify).await;
}
