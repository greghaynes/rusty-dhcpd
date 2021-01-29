mod config;
mod server;

use dhcp4r::server as dhcp4rserver;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;

const SERVER_IP: Ipv4Addr = Ipv4Addr::new(10, 40, 4, 122);
const SERVER_PORT: u16 = 8089;

fn main() {
    let config = config::Config {
        bind_address: SocketAddrV4::new(SERVER_IP, SERVER_PORT),
        lease_start: Ipv4Addr::new(10, 41, 0, 0),
        lease_count: 24,
        lease_duration: Duration::from_secs(300),
        lease_subnet_mask: Ipv4Addr::new(255, 255, 255, 0),
        lease_routers: vec![Ipv4Addr::new(10, 41, 1, 1)],
        lease_domain_servers: vec![Ipv4Addr::new(8, 8, 8, 8)],
    };
    let srv = server::Server::serve(&config);
}
