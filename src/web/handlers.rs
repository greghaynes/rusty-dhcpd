use crate::dhcpd::leases;
use crate::dhcpd::server::AbstractServer;
use crate::web::schemas;
use std::convert::From;
use std::convert::Infallible;
use std::sync::Arc;
use warp::Filter;

impl From<&leases::LeaseBlock> for schemas::LeaseBlock {
    fn from(dhcpd_block: &leases::LeaseBlock) -> Self {
        schemas::LeaseBlock {
            start_address: dhcpd_block.start_address,
            max_leases: dhcpd_block.max_leases,
            lease_duration: dhcpd_block.lease_duration,
            subnet_mask: dhcpd_block.subnet_mask,
            routers: dhcpd_block.routers.clone(),
            domain_servers: dhcpd_block.domain_servers.clone(),
            leases: None,
        }
    }
}

pub async fn leases_handler(
    dhcpd: Arc<dyn AbstractServer>,
) -> Result<impl warp::Reply, Infallible> {
    let mut leases = schemas::Leases::new();
    for lease_block in dhcpd.leases() {
        let mut web_block = schemas::LeaseBlock::from(lease_block);
        web_block.leases = Some(lease_block.leases().await);
        leases.push(web_block);
    }
    return Ok(warp::reply::json(&leases));
}

fn with_dhcpd(
    dhcpd: Arc<dyn AbstractServer>,
) -> impl Filter<Extract = (Arc<dyn AbstractServer>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || dhcpd.clone())
}

pub fn filters(
    dhcpd: Arc<dyn AbstractServer>,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path("leases")
        .and(with_dhcpd(dhcpd))
        .and_then(leases_handler)
}
