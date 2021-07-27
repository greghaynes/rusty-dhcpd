use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

#[derive(Clone)]
pub struct Config {
    pub server_ip: Ipv4Addr,
    pub listen_port: u16,
    pub bind_interface: Option<String>,
    pub lease_start: Ipv4Addr,
    pub lease_count: u32,
    pub lease_duration: Duration,
    pub lease_subnet_mask: Ipv4Addr,
    pub lease_routers: Vec<Ipv4Addr>,
    pub lease_domain_servers: Vec<Ipv4Addr>,
}
