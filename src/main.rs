mod config;
mod server;

use dhcp4r::server as dhcp4rserver;
use std::net::{Ipv4Addr, UdpSocket};

const SERVER_IP: Ipv4Addr = Ipv4Addr::new(10, 40, 4, 122);

fn main() {
    let socket = UdpSocket::bind("0.0.0.0:67").unwrap();
    socket.set_broadcast(true).unwrap();

    let config = server::ServerConfig {
        lease_start: Ipv4Addr::new(10, 41, 0, 0),
    };
    let srv = server::Server { config: config };

    dhcp4rserver::Server::serve(socket, SERVER_IP, srv);
}
