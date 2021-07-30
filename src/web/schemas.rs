use crate::dhcpd::leases::Lease;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Duration;

use serde_derive::Serialize;

#[derive(Serialize)]
pub struct LeaseBlock {
    // Start of lease block
    pub start_address: Ipv4Addr,
    // Total number of leases
    pub max_leases: u32,
    pub lease_duration: Duration,
    pub subnet_mask: Ipv4Addr,
    pub routers: Vec<Ipv4Addr>,
    pub domain_servers: Vec<Ipv4Addr>,

    pub leases: Option<HashMap<Ipv4Addr, Lease>>,
}

pub type Leases = Vec<LeaseBlock>;
