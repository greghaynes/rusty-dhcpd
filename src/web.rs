use crate::dhcpd::{ AbstractServer, Server };
use std::convert::Infallible;
use std::sync::Arc;

pub async fn leases_handler(dhcpd: Arc<Server>) -> Result<impl warp::Reply, Infallible> {
    return Ok(warp::reply::json(&(
        dhcpd.leases().await
    )));
}
