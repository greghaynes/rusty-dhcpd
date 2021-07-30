use std::net::Ipv4Addr;
use crate::dhcpd::leases::Lease;
use std::collections::HashMap;
use std::time::Duration;

struct LeaseBlock {
    // Start of lease block
    pub start_address: Ipv4Addr,
    // Total number of leases
    pub max_leases: u32,
    pub lease_duration: Duration,
    pub subnet_mask: Ipv4Addr,
    pub routers: Vec<Ipv4Addr>,
    pub domain_servers: Vec<Ipv4Addr>,

    pub leases: HashMap<Ipv4Addr, Lease>,
}

type Leases = Vec<LeaseBlock>;