use crate::config::Config;
use serde_derive::Serialize;
use std::collections::HashMap;
use std::convert::From;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Debug)]
pub enum LeaseError {
    AddressUnavailabe(),
    IpChaddrMismatch(),
}

impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match &*self {
            LeaseError::AddressUnavailabe() => write!(f, "Address is unavailable"),
            LeaseError::IpChaddrMismatch() => write!(f, "Address does not match hw address"),
        }
    }
}

#[derive(Clone, Serialize)]
pub struct Lease {
    pub chaddr: [u8; 6],
    #[serde(with = "serde_millis")]
    pub expires: Instant,
}

impl Lease {
    pub fn available(&self, chaddr: &[u8; 6]) -> bool {
        if self.expired() {
            return false;
        }

        return *chaddr == self.chaddr;
    }

    pub fn expired(&self) -> bool {
        return self.expires <= Instant::now();
    }
}

struct LeaseBlockState {
    last_lease: u32,
    lease_by_ip: HashMap<Ipv4Addr, Lease>,
    ip_by_chaddr: HashMap<[u8; 6], Ipv4Addr>,
}

impl LeaseBlockState {
    fn available(&self, chaddr: &[u8; 6], addr: &Ipv4Addr) -> bool {
        match self.lease_by_ip.get(addr) {
            Some(lease) => {
                return lease.available(chaddr);
            }
            None => return true,
        }
    }

    fn reserve(
        &mut self,
        chaddr: &[u8; 6],
        addr: &Ipv4Addr,
        duration: Duration,
    ) -> Result<(), LeaseError> {
        if !self.available(chaddr, addr) {
            return Err(LeaseError::AddressUnavailabe());
        }

        match self.lease_by_ip.get_mut(addr) {
            Some(lease) => {
                lease.expires = Instant::now() + duration;
            }
            None => {
                self.lease_by_ip.insert(
                    *addr,
                    Lease {
                        chaddr: *chaddr,
                        expires: Instant::now() + duration,
                    },
                );
            }
        }

        self.ip_by_chaddr.insert(*chaddr, *addr);
        Ok(())
    }
}

/// Operations on a contiguous set of IP addresses which are used for leases
///
/// Lease range begins at start_address, ends at start_address + max_leases
pub struct LeaseBlock {
    // Start of lease block
    pub start_address: Ipv4Addr,
    // Total number of leases
    pub max_leases: u32,
    pub lease_duration: Duration,
    pub subnet_mask: Ipv4Addr,
    pub routers: Vec<Ipv4Addr>,
    pub domain_servers: Vec<Ipv4Addr>,

    state: RwLock<LeaseBlockState>,
}

impl LeaseBlock {
    pub fn create(
        start_address: Ipv4Addr,
        max_leases: u32,
        lease_duration: Duration,
        subnet_mask: Ipv4Addr,
        routers: Vec<Ipv4Addr>,
        domain_servers: Vec<Ipv4Addr>,
    ) -> LeaseBlock {
        return LeaseBlock {
            start_address: start_address,
            max_leases: max_leases,
            lease_duration: lease_duration,
            subnet_mask: subnet_mask,
            routers: routers,
            domain_servers: domain_servers,
            state: RwLock::new(LeaseBlockState {
                last_lease: 0,
                lease_by_ip: HashMap::new(),
                ip_by_chaddr: HashMap::new(),
            }),
        };
    }

    pub async fn available(&self, chaddr: &[u8; 6], addr: &Ipv4Addr) -> bool {
        let state_guard = self.state.read().await;
        return state_guard.available(chaddr, addr);
    }

    pub async fn next_available(&self, chaddr: &[u8; 6]) -> Option<Ipv4Addr> {
        let state_guard = self.state.read().await;
        let mut cur_offset = state_guard.last_lease;
        loop {
            let cur_address =
                Ipv4Addr::from(u32::from(self.start_address) + (cur_offset % self.max_leases));
            if state_guard.available(chaddr, &cur_address) {
                return Some(cur_address);
            }
            cur_offset += 1;
            if (cur_offset % self.max_leases) == state_guard.last_lease {
                return None;
            }
        }
    }

    pub async fn get_available(&self, chaddr: &[u8; 6]) -> Option<Ipv4Addr> {
        let state_guard = self.state.read().await;
        match state_guard.ip_by_chaddr.get(chaddr) {
            Some(address) => {
                return Some(*address);
            }
            None => {
                return self.next_available(chaddr).await;
            }
        }
    }

    pub async fn reserve(&self, chaddr: &[u8; 6], addr: &Ipv4Addr) -> Result<(), LeaseError> {
        let mut state_guard = self.state.write().await;
        return state_guard.reserve(chaddr, addr, self.lease_duration);
    }

    pub fn release(&self, chaddr: &[u8; 6], addr: &Ipv4Addr) -> Result<(), LeaseError> {
        Ok(())
    }

    pub async fn leases<'a>(&self) -> HashMap<Ipv4Addr, Lease> {
        let state_guard = self.state.read().await;
        return state_guard.lease_by_ip.clone();
    }
}

impl From<&Config> for LeaseBlock {
    fn from(config: &Config) -> LeaseBlock {
        LeaseBlock::create(
            config.lease_start,
            config.lease_count,
            config.lease_duration,
            config.lease_subnet_mask,
            config.lease_routers.clone(),
            config.lease_domain_servers.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::time::{Duration, Instant};

    fn create_block() -> super::LeaseBlock {
        return super::LeaseBlock::create(
            Ipv4Addr::new(10, 10, 0, 10),
            50,
            Duration::from_secs(100),
            Ipv4Addr::new(255, 255, 255, 0),
            vec![Ipv4Addr::new(10, 10, 0, 1)],
            vec![Ipv4Addr::new(8, 8, 8, 8)],
        );
    }

    #[tokio::test]
    async fn next_available() {
        let mut chaddr: [u8; 6] = [0; 6];
        let block = create_block();
        match block.next_available(&chaddr).await {
            Some(addr) => assert_eq!(addr, Ipv4Addr::new(10, 10, 0, 10)),
            None => assert!(false, "Expected IP address but got none"),
        }

        {
            let mut lease_guard = block.state.write().await;
            // Make sure we can re lease if our chaddr matches
            lease_guard.lease_by_ip.insert(
                Ipv4Addr::new(10, 10, 0, 10),
                super::Lease {
                    chaddr: chaddr,
                    expires: Instant::now() + Duration::from_secs(10),
                },
            );
        }
        match block.next_available(&chaddr).await {
            Some(addr) => assert_eq!(addr, Ipv4Addr::new(10, 10, 0, 10)),
            None => assert!(false, "Expected IP address but got none"),
        }

        // Make sure we get next IP when chaddr doesnt match
        chaddr = [1; 6];
        match block.next_available(&chaddr).await {
            Some(addr) => assert_eq!(addr, Ipv4Addr::new(10, 10, 0, 11)),
            None => assert!(false, "Expected IP address but got none"),
        }

        // Check wrapping when we hit max_leases
        {
            let mut lease_guard = block.state.write().await;
            lease_guard.last_lease = 50;
        }
        match block.next_available(&chaddr).await {
            Some(addr) => assert_eq!(addr, Ipv4Addr::new(10, 10, 0, 11)),
            None => assert!(false, "Expected IP address but got none"),
        }
    }
}
