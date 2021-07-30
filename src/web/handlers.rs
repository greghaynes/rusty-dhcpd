use crate::dhcpd::server::AbstractServer;
use std::convert::Infallible;
use std::sync::Arc;

pub async fn leases_handler(
    dhcpd: Arc<dyn AbstractServer>,
) -> Result<impl warp::Reply, Infallible> {
    let mut leases = Vec::new();
    for lease_block in dhcpd.leases() {
        leases.push(lease_block.leases().await);
    }
    return Ok(warp::reply::json(&leases));
}
