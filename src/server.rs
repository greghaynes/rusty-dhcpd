use dhcp4r::{options, packet};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};
use tokio::io::Error as TokioError;
use tokio::net::UdpSocket;

use crate::config;

#[derive(Debug)]
enum HandleError {
    IOError(std::io::Error),
    PacketError(),
}

impl std::fmt::Display for HandleError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            HandleError::IOError(err) => write!(f, "IOError: {}", err),
            HandleError::PacketError() => write!(f, "Failed parsing packet"),
        }
    }
}

impl From<std::io::Error> for HandleError {
    fn from(error: std::io::Error) -> Self {
        return HandleError::IOError(error);
    }
}

#[derive(Debug)]
enum ListenError {
    BindError(TokioError),
    HandleError(HandleError),
}

impl std::fmt::Display for ListenError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            ListenError::BindError(tokio_err) => write!(f, "Bind Error: {}", tokio_err),
            ListenError::HandleError(err) => write!(f, "Packet Receive Error: {}", err),
        }
    }
}

pub struct Lease {
    chaddr: [u8; 6],
    expires: Instant,
}

impl Lease {
    pub fn expired(&self) -> bool {
        return self.expires <= Instant::now();
    }
}

pub struct LeaseBlock {
    start_address: Ipv4Addr,
    max_leases: u32,
    last_lease: u32,
    lease_duration: Duration,
    leases: HashMap<Ipv4Addr, Lease>,
}

impl LeaseBlock {
    pub fn available(&self, chaddr: &[u8; 6], addr: &Ipv4Addr) -> bool {
        match self.leases.get(addr) {
            Some(lease) => return !lease.expired(),
            None => return false,
        }
    }
}

pub struct Server {
    config: config::Config,
    lease_block: LeaseBlock,
    socket: UdpSocket,
    out_buff: [u8; 1500],
}

impl Server {
    pub async fn serve(config: &config::Config) -> Result<(), ListenError> {
        let sock_result = UdpSocket::bind(config.bind_address).await;
        let socket: UdpSocket;
        match sock_result {
            Err(err) => return Err(ListenError::BindError(err)),
            Ok(sock) => {
                socket = sock;
            }
        }

        let server = Server {
            config: config.clone(),
            lease_block: LeaseBlock {
                start_address: config.lease_start,
                max_leases: config.lease_count,
                last_lease: 0,
                lease_duration: config.lease_duration,
                leases: HashMap::new(),
            },
            socket: socket,
            out_buff: [0; 1500],
        };

        match server.recv_loop(&server.socket).await {
            Err(err) => return Err(ListenError::HandleError(err)),
            Ok(()) => return Ok(()),
        }
    }

    async fn recv_loop(&self, sock: &UdpSocket) -> Result<(), HandleError> {
        loop {
            sock.readable().await?;

            let mut buf = [0; 1500];

            match sock.try_recv_from(&mut buf) {
                Ok((n, addr)) => match dhcp4r::packet::Packet::from(&buf[..n]) {
                    Err(err) => {
                        return Err(HandleError::PacketError());
                    }
                    Ok(packet) => return self.handle_packet(&packet, &addr),
                },
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    continue;
                }
                Err(e) => {
                    return Err(HandleError::IOError(e));
                }
            }
        }

        Ok(())
    }

    fn handle_packet(
        &self,
        packet: &dhcp4r::packet::Packet,
        src: &std::net::SocketAddr,
    ) -> Result<(), HandleError> {
        match packet.message_type() {
            Ok(options::MessageType::Discover) => {
                // Client requested an address
                if let Some(options::DhcpOption::RequestedIpAddress(addr)) =
                    packet.option(options::REQUESTED_IP_ADDRESS)
                {
                    if self.lease_block.available(&packet.chaddr, addr) {}
                }
                return Ok(());
            }

            Ok(options::MessageType::Request) => return Ok(()),
        }
    }

    async fn send(
        &mut self,
        src: &std::net::SocketAddr,
        packet: packet::Packet,
    ) -> std::io::Result<usize> {
        let mut addr = *src;
        if packet.broadcast || addr.ip() == IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)) {
            addr.set_ip(IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255)))
        }
        self.socket
            .send_to(packet.encode(&mut self.out_buff), addr)
            .await
    }

    async fn reply(
        &mut self,
        src: &std::net::SocketAddr,
        msg_type: options::MessageType,
        packet: packet::Packet,
        offer_ip: &Ipv4Addr,
        options: Vec<options::DhcpOption>,
    ) -> std::io::Result<usize> {
        let ciaddr = match msg_type {
            options::MessageType::Nak => Ipv4Addr::new(0, 0, 0, 0),
            _ => packet.ciaddr,
        };

        let mut opts: Vec<options::DhcpOption> = Vec::with_capacity(options.len() + 2);
        opts.push(options::DhcpOption::DhcpMessageType(msg_type));
        opts.push(options::DhcpOption::ServerIdentifier(
            *self.config.bind_address.ip(),
        ));

        self.send(
            src,
            packet::Packet {
                reply: true,
                hops: 0,
                xid: packet.xid,
                secs: 0,
                broadcast: packet.broadcast,
                ciaddr: ciaddr,
                yiaddr: *offer_ip,
                siaddr: Ipv4Addr::new(0, 0, 0, 0),
                giaddr: packet.giaddr,
                chaddr: packet.chaddr,
                options: opts,
            },
        )
        .await
    }
}
