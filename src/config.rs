use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

#[derive(Clone)]
pub struct Config {
    pub bind_address: SocketAddrV4,
    pub lease_start: Ipv4Addr,
    pub lease_count: u32,
    pub lease_duration: Duration,
}
